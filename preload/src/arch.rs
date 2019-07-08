#[cfg(target_arch = "x86")]
pub const TARGET_ARCH: &str = "x86";

#[cfg(target_arch = "x86_64")]
pub const TARGET_ARCH: &str = "x86_64";

#[cfg(target_arch = "mips")]
pub const TARGET_ARCH: &str = "mips";

#[cfg(target_arch = "mips64")]
pub const TARGET_ARCH: &str = "mips64";

#[cfg(target_arch = "powerpc")]
pub const TARGET_ARCH: &str = "powerpc";

#[cfg(target_arch = "powerpc64")]
pub const TARGET_ARCH: &str = "powerpc64";

#[cfg(target_arch = "arm")]
pub const TARGET_ARCH: &str = "arm";

#[cfg(target_arch = "aarch64")]
pub const TARGET_ARCH: &str = "aarch64";

#[cfg(target_endian = "little")]
pub const IS_LITTLE_ENDIAN: bool = true;

#[cfg(target_endian = "big")]
pub const IS_LITTLE_ENDIAN: bool = false;
