#![no_std]

//! Linux raw-syscall access to the LXCFS CPU view.

use core::ffi::CStr;

use cpuhint_core::{CpuList, parse_cpu_list};
use rustix::{
    fs::{Mode, OFlags, open},
    io::{Errno, read},
};

const ONLINE_CPU_PATH: &CStr = c"/sys/devices/system/cpu/online";
const MAX_ONLINE_BYTES: usize = 128;

/// Why the LXCFS CPU view could not be resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveError {
    Open,
    Read,
    TooLarge,
    Invalid,
}

/// Reads and parses LXCFS's canonical online CPU list.
///
/// The descriptor is not cached so each invocation can observe an entitlement
/// change made by LXCFS.
pub fn resolve_online_cpu_list() -> Result<CpuList, ResolveError> {
    let fd = open(
        ONLINE_CPU_PATH,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ResolveError::Open)?;

    let mut buffer = [0; MAX_ONLINE_BYTES];
    let length = read_retrying_interrupts(&fd, &mut buffer).map_err(|_| ResolveError::Read)?;
    if length == buffer.len() {
        let mut extra = [0];
        if read_retrying_interrupts(&fd, &mut extra).map_err(|_| ResolveError::Read)? != 0 {
            return Err(ResolveError::TooLarge);
        }
    }

    parse_cpu_list(&buffer[..length]).map_err(|_| ResolveError::Invalid)
}

fn read_retrying_interrupts(fd: impl rustix::fd::AsFd, buffer: &mut [u8]) -> Result<usize, Errno> {
    loop {
        match read(&fd, &mut *buffer) {
            Err(Errno::INTR) => continue,
            result => return result,
        }
    }
}
