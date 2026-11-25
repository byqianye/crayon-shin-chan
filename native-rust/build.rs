fn main() {
    println!("cargo:rerun-if-changed=../build/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../build/icon.ico")
            .set("ProductName", "蜡笔小新")
            .set("FileDescription", "蜡笔小新 Rust 桌面客户端")
            .set("InternalName", "crayon-rust")
            .set("OriginalFilename", "crayon-rust.exe")
            .set("LegalCopyright", "Copyright © 2026 byqianye")
            .compile()
            .expect("compile Windows icon and version resources");
    }
}
