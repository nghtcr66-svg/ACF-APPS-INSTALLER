fn main() {
    // 1. Компиляция интерфейса Slint
    slint_build::compile("src/installer.slint").expect("Slint compilation failed");
    
    // 2. Ресурсы для Windows (Иконка + Авто-Администратор)
    if std::env::var("TARGET").unwrap().contains("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("alfa.ico"); // Ваша иконка в корне проекта
        
        // ПОДКЛЮЧАЕМ МАНИФЕСТ: зашиваем требование прав админа в бинарник
        res.set_manifest_file("installer.manifest"); 
        
        res.compile().unwrap();
    }
}
