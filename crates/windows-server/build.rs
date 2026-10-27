use std::path::Path;

/// The Windows resources: the Mujin Music icon and the version block that
/// Explorer, Task Manager and the firewall prompt show as the program's
/// name. Generated so the version always matches this crate's.
fn resource_script(icon: &Path, version: &str) -> String {
    let numbers: Vec<u16> = version
        .split(['.', '-', '+'])
        .take(3)
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    let [major, minor, patch] = [0, 1, 2].map(|index| numbers.get(index).copied().unwrap_or(0));
    let icon = icon.display().to_string().replace('\\', "\\\\");
    format!(
        r#"#define APP_ICON 1
APP_ICON ICON "{icon}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3F
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Mujin Music"
      VALUE "FileDescription", "Mujin Music Server"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "ParsonMusicServer"
      VALUE "LegalCopyright", "GPL-3.0-only. Based on Parson by ParsonLabs."
      VALUE "OriginalFilename", "ParsonMusicServer.exe"
      VALUE "ProductName", "Mujin Music"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    )
}

fn main() {
    println!("cargo:rerun-if-changed=parson.ico");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let script = Path::new(&out_dir).join("app.rc");
    std::fs::write(
        &script,
        resource_script(&Path::new(&manifest_dir).join("parson.ico"), &version),
    )
    .expect("write the Windows resource script");
    if std::env::var("TARGET").is_ok_and(|target| target.contains("windows")) {
        embed_resource::compile(&script, embed_resource::NONE)
            .manifest_required()
            .expect("compile Mujin Music for Windows resources");
    }
}
