fn main() {
    // 1. Компиляция интерфейса Slint
    slint_build::compile("src/installer.slint").expect("Slint compilation failed");
    
    // 2. Иконка для Windows
    if std::env::var("TARGET").unwrap().contains("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("alfa.ico");
        let _ = res.compile().unwrap();
    }
}
