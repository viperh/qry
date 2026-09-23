use anyhow::Result;
use vergen_gix::{Build, Cargo, Emitter, Gix};
use winresource::WindowsResource;

fn main() -> Result<()> {
    let build = Build::all_build();
    let gix = Gix::all_git();
    let cargo = Cargo::all_cargo();
    Emitter::default()
        .default_on_error()
        .add_instructions(&build)?
        .add_instructions(&gix)?
        .add_instructions(&cargo)?
        .emit()?;

    println!("cargo:rerun-if-changed=assets/icon.ico");


    if std::env::var("CARGO_CFG_TARGET_OS")? == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "MyTool")
            .set("FileDescription", "MyTool command-line utility")
            .set("LegalCopyright", "© 2026 Viper");
        res.compile()?;
    }

    Ok(())
}
