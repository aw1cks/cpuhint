#![cfg_attr(all(target_os = "linux", not(test)), no_std)]

//! Narrow glibc interposition for the LXCFS CPU view.

#[cfg(target_os = "linux")]
mod linux {
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!("cpuhint-preload supports only x86_64 and aarch64 Linux");

    #[cfg(target_arch = "x86_64")]
    core::arch::global_asm!(
        ".globl rust_eh_personality",
        ".hidden rust_eh_personality",
        ".type rust_eh_personality,@function",
        "rust_eh_personality:",
        "mov $8, %eax",
        "ret",
        ".size rust_eh_personality, .-rust_eh_personality",
        options(att_syntax),
    );

    #[cfg(target_arch = "aarch64")]
    core::arch::global_asm!(
        ".globl rust_eh_personality",
        ".hidden rust_eh_personality",
        ".type rust_eh_personality,%function",
        "rust_eh_personality:",
        "mov w0, #8",
        "ret",
        ".size rust_eh_personality, .-rust_eh_personality",
    );

    use core::{
        ffi::{c_int, c_long, c_ulong, c_void},
        ptr,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use cpuhint_core::build_contiguous_mask;
    use cpuhint_linux::resolve_online_cpu_list;

    type SysconfFn = unsafe extern "C" fn(c_int) -> c_long;
    type GetNprocsFn = unsafe extern "C" fn() -> c_int;

    const RESOLVING: usize = 1;
    const UNAVAILABLE: usize = 2;

    static SYSCONF: AtomicUsize = AtomicUsize::new(0);
    static GET_NPROCS: AtomicUsize = AtomicUsize::new(0);
    static GET_NPROCS_CONF: AtomicUsize = AtomicUsize::new(0);

    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
        fn getauxval(kind: c_ulong) -> c_ulong;
        fn __errno_location() -> *mut c_int;
    }

    /// Interposes glibc's process CPU count queries.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn sysconf(name: c_int) -> c_long {
        if is_secure_execution() || !is_cpu_count_query(name) {
            return call_sysconf(name);
        }

        match resolve_online_cpu_list() {
            Ok(list) => list.count() as c_long,
            Err(_) => call_sysconf(name),
        }
    }

    /// Interposes glibc's online CPU count helper.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn get_nprocs() -> c_int {
        if is_secure_execution() {
            return call_get_nprocs(&GET_NPROCS, c"get_nprocs");
        }

        match resolve_online_cpu_list() {
            Ok(list) => list.count() as c_int,
            Err(_) => call_get_nprocs(&GET_NPROCS, c"get_nprocs"),
        }
    }

    /// Interposes glibc's configured CPU count helper.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn get_nprocs_conf() -> c_int {
        if is_secure_execution() {
            return call_get_nprocs(&GET_NPROCS_CONF, c"get_nprocs_conf");
        }

        match resolve_online_cpu_list() {
            Ok(list) => list.count() as c_int,
            Err(_) => call_get_nprocs(&GET_NPROCS_CONF, c"get_nprocs_conf"),
        }
    }

    /// Interposes the calling thread's affinity query only.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn sched_getaffinity(
        pid: libc::pid_t,
        cpusetsize: libc::size_t,
        mask: *mut libc::cpu_set_t,
    ) -> c_int {
        let kernel_bytes = match raw_sched_getaffinity(pid, cpusetsize, mask.cast()) {
            Ok(bytes) => bytes,
            Err(errno) => {
                set_errno(errno);
                return -1;
            }
        };

        // glibc clears bytes beyond the kernel's returned mask size on success.
        unsafe {
            ptr::write_bytes(
                mask.cast::<u8>().add(kernel_bytes),
                0,
                cpusetsize - kernel_bytes,
            )
        };

        if pid != 0 || is_secure_execution() {
            return 0;
        }

        let real_mask: &[u8] =
            unsafe { core::slice::from_raw_parts(mask.cast::<u8>(), kernel_bytes) };
        let real_count = real_mask
            .iter()
            .map(|byte| byte.count_ones() as usize)
            .sum::<usize>();
        let Ok(view) = resolve_online_cpu_list() else {
            return 0;
        };
        if !view.is_contiguous_from_zero() {
            return 0;
        }

        let count = view.count().min(real_count);
        let mask = unsafe { core::slice::from_raw_parts_mut(mask.cast::<u8>(), cpusetsize) };
        // `count` is bounded by the successfully returned kernel mask.
        let _ = build_contiguous_mask(mask, count);
        0
    }

    fn is_cpu_count_query(name: c_int) -> bool {
        name == libc::_SC_NPROCESSORS_CONF || name == libc::_SC_NPROCESSORS_ONLN
    }

    fn is_secure_execution() -> bool {
        unsafe { getauxval(libc::AT_SECURE as c_ulong) != 0 }
    }

    fn call_sysconf(name: c_int) -> c_long {
        let Some(pointer) = resolve_symbol(&SYSCONF, c"sysconf") else {
            set_errno(libc::ENOSYS);
            return -1;
        };
        let function: SysconfFn = unsafe { core::mem::transmute(pointer) };
        unsafe { function(name) }
    }

    fn call_get_nprocs(slot: &AtomicUsize, symbol: &core::ffi::CStr) -> c_int {
        let Some(pointer) = resolve_symbol(slot, symbol) else {
            set_errno(libc::ENOSYS);
            return -1;
        };
        let function: GetNprocsFn = unsafe { core::mem::transmute(pointer) };
        unsafe { function() }
    }

    fn resolve_symbol(slot: &AtomicUsize, symbol: &core::ffi::CStr) -> Option<usize> {
        match slot.compare_exchange(0, RESOLVING, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                let saved_errno = get_errno();
                let pointer = unsafe { dlsym(libc::RTLD_NEXT, symbol.as_ptr()) } as usize;
                set_errno(saved_errno);
                let state = if pointer == 0 { UNAVAILABLE } else { pointer };
                slot.store(state, Ordering::Release);
                (pointer != 0).then_some(pointer)
            }
            Err(RESOLVING | UNAVAILABLE) => None,
            Err(pointer) => Some(pointer),
        }
    }

    fn set_errno(errno: c_int) {
        unsafe { *__errno_location() = errno }
    }

    fn get_errno() -> c_int {
        unsafe { *__errno_location() }
    }

    fn raw_sched_getaffinity(
        pid: libc::pid_t,
        cpusetsize: libc::size_t,
        mask: *mut u8,
    ) -> Result<usize, c_int> {
        let size = cpusetsize.min(c_int::MAX as usize);
        let result = unsafe { sched_getaffinity_syscall(pid, size, mask) };
        if result < 0 {
            let errno = result.unsigned_abs();
            return Err(if errno <= 4095 {
                errno as c_int
            } else {
                libc::EIO
            });
        }
        let bytes = result as usize;
        if bytes > size {
            return Err(libc::EIO);
        }
        Ok(bytes)
    }

    #[cfg(target_arch = "x86_64")]
    unsafe fn sched_getaffinity_syscall(
        pid: libc::pid_t,
        cpusetsize: usize,
        mask: *mut u8,
    ) -> isize {
        let result: isize;
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") libc::SYS_sched_getaffinity as isize => result,
                in("rdi") pid as isize,
                in("rsi") cpusetsize,
                in("rdx") mask,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
        result
    }

    #[cfg(target_arch = "aarch64")]
    unsafe fn sched_getaffinity_syscall(
        pid: libc::pid_t,
        cpusetsize: usize,
        mask: *mut u8,
    ) -> isize {
        let result: isize;
        unsafe {
            core::arch::asm!(
                "svc 0",
                inlateout("x8") libc::SYS_sched_getaffinity as isize => _,
                inlateout("x0") pid as isize => result,
                in("x1") cpusetsize,
                in("x2") mask,
                options(nostack),
            );
        }
        result
    }

    #[cfg(not(test))]
    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("ud2", options(noreturn));
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("brk #0", options(noreturn));
        }
        #[allow(unreachable_code)]
        loop {
            core::hint::spin_loop();
        }
    }
}
