pub const TARGET_ARCH: &str = std::env::consts::ARCH;

#[cfg(target_endian = "little")]
pub const IS_LITTLE_ENDIAN: bool = true;

#[cfg(target_endian = "big")]
pub const IS_LITTLE_ENDIAN: bool = false;
