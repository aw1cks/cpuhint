fn main() {
    // The preload DSO has no runtime initialization; avoid CRT startup objects.
    println!("cargo::rustc-link-arg-cdylib=-nostartfiles");
}
