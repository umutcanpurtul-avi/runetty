pub const APP_ID: &str = "io.github.umutcanpurtul_avi.runetty";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// GResource root path. Must match the prefix in gresource.xml.
pub const RESOURCE_PATH: &str = "/io/github/umutcanpurtul_avi/runetty";

/// Build floor. The matrix in BUILDING.md is keyed to these three.
pub const MIN_GTK: (u32, u32) = (4, 14);
pub const MIN_ADW: (u32, u32) = (1, 5);
