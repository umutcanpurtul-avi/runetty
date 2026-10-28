pub const APP_ID: &str = "dev.runetty.Runetty";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// GResource kok yolu. gresource.xml icindeki prefix ile ayni olmali.
pub const RESOURCE_PATH: &str = "/dev/runetty/Runetty";

/// Derleme tabani. BUILDING.md'deki matris bu uclu ile eslesir.
pub const MIN_GTK: (u32, u32) = (4, 14);
pub const MIN_ADW: (u32, u32) = (1, 5);
