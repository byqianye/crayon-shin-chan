#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        #[cfg(feature = "desktop")]
        rfd::MessageDialog::new()
            .set_title("蜡笔小新")
            .set_description(format!("{error:#}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let mut directory = std::env::var_os("CRAYON_DATA_DIR").map(std::path::PathBuf::from);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--data-dir" => {
                directory = Some(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--data-dir 缺少目录"))?
                        .into(),
                )
            }
            "--version" => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => anyhow::bail!("未知参数：{argument}"),
        }
    }
    let directory = directory.unwrap_or(
        std::env::current_exe()?
            .parent()
            .ok_or_else(|| anyhow::anyhow!("无法定位程序目录"))?
            .join("data"),
    );
    let store = std::sync::Arc::new(crayon_core::storage::Store::open(directory)?);
    #[cfg(feature = "desktop")]
    crayon_core::ui::launch(store)?;
    #[cfg(not(feature = "desktop"))]
    {
        let _ = store;
        anyhow::bail!("桌面界面需要 desktop feature");
    }
    #[allow(unreachable_code)]
    Ok(())
}
