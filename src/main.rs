#![windows_subsystem = "windows"]
use include_dir::{include_dir, Dir};
use notify_rust::Notification;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use serde;



slint::include_modules!();

// 1. Зашиваем папку "123" целиком внутрь бинарника
static PROJECT_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/123");

// 2. Вшиваем установщик .NET 8 прямо в память бинарника (Offline-установка)
// Файл dotnet_installer.exe должен лежать в корне проекта рядом с Cargo.toml
const DOTNET_INSTALLER_BYTES: &[u8] = std::include_bytes!(concat!(std::env!("CARGO_MANIFEST_DIR"), "/dninst.exe"));

// ФУНКЦИЯ ПРОВЕРКИ ТЕГА РЕЛИЗА
fn check_release_tag(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let response = ureq::get(url)
        .set("User-Agent", "Slint-Installer-App")
        .timeout(Duration::from_secs(5)) 
        .call()?;
    
    let json_text = response.into_string()?;
    let json_parsed: Value = serde_json::from_str(&json_text)?;
    
    if let Some(tag) = json_parsed.get("tag_name") {
        if let Some(tag_str) = tag.as_str() {
            return Ok(tag_str.to_string());
        }
    }
    
    Ok(json_text.trim().to_string())
}

// ОБНОВЛЕННАЯ ФУНКЦИЯ ОФФЛАЙН-УСТАНОВКИ .NET 8 ИЗ ПАМЯТИ БИНАРНИКА
fn auto_install_net8_offline() -> Result<(), Box<dyn std::error::Error>> {
    println!("Извлечение локального установщика .NET 8 из памяти...");
    
    let temp_dir = std::env::temp_dir();
    let installer_path = temp_dir.join("local_dotnet_8_installer.exe");
    
    // Записываем вшитые байты во временный файл на диске
    fs::write(&installer_path, DOTNET_INSTALLER_BYTES)?;
    
    println!("Установщик извлечен во временную директорию. Запуск тихой установки...");
    
    // Запускаем процесс тихой установки без вызова сторонних окон
    let status = Command::new(&installer_path)
        .arg("/install")
        .arg("/quiet")
        .arg("/norestart")
        .status()?; // блокирует поток и ждет завершения
        
    if status.success() {
        println!(".NET 8 успешно установлен из локальных ресурсов бинарника.");
    } else {
        eprintln!("Локальный установщик .NET 8 завершился с кодом ошибки.");
    }
    
    // Подчищаем за собой временный файл
    let _ = fs::remove_file(installer_path);
    Ok(())
}

fn show_notification_net8(
    summary: &str,
    body: &str,
    link: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    Notification::new()
        .summary(summary)
        .body(body)
        .icon("dialog-information")
        .action("open_site", "Перейти на сайт")
        .show()?
        .wait_for_action(|action| {
            if action == "open_site" {
                println!("Открываем сайт...");
                if let Err(err) = open::that(link) {
                    eprintln!("Не удалось открыть ссылку: {}", err);
                }
            }
        });
    Ok(())
}

fn is_net8_installed() -> bool {
    let output = match Command::new("dotnet").arg("--list-runtimes").output() {
        Ok(out) => out,
        Err(_) => return false,
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|line| {
        line.starts_with("Microsoft.NETCore.App 8.0")
            || line.starts_with("Microsoft.AspNetCore.App 8.0")
    })
}

// ФУНКЦИЯ РАСПАКОВКИ ИЗ ПАМЯТИ (ДЛЯ ПАПКИ 123)
fn extract_embedded_dir(
    target_dir: &str,
    ui_weak: slint::Weak<MainWindow>,
) -> Result<(), std::io::Error> {
    let dst_path = Path::new(target_dir);
    fs::create_dir_all(dst_path)?;

    let files: Vec<_> = PROJECT_DIR.files().collect();
    let total_files = files.len();

    if total_files == 0 {
        return Ok(());
    }

    for (index, embedded_file) in files.iter().enumerate() {
        let relative_path = embedded_file.path();
        let current_target = dst_path.join(relative_path);

        if let Some(file_name) = current_target.file_name() {
            let status_text = slint::SharedString::from(format!(
                "Развёртывание: {}", 
                file_name.to_string_lossy()
            ));
            let _ = slint::invoke_from_event_loop({
                let ui_clone = ui_weak.clone();
                move || {
                    if let Some(ui) = ui_clone.upgrade() {
                        ui.set_current_status_text(status_text);
                    }
                }
            });
        }

        if let Some(parent_dir) = current_target.parent() {
            fs::create_dir_all(parent_dir)?;
        }

        fs::write(&current_target, embedded_file.contents())?;

        let progress = (index + 1) as f32 / total_files as f32;
        let _ = slint::invoke_from_event_loop({
            let ui_clone = ui_weak.clone();
            move || {
                if let Some(ui) = ui_clone.upgrade() {
                    ui.set_progress_value(progress);
                }
            }
        });

        std::thread::sleep(Duration::from_millis(25));
    }

    Ok(())
}


// Структура для сохранения версии в JSON
#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct AppConfig {
    last_known_version: String,
}

// ФУНКЦИЯ ПРОВЕРКИ ТЕГА РЕЛИЗА И ОБНОВЛЕНИЯ JSON В APPDATA
fn check_and_update_version_system(url: &str, github_release_page: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Получаем путь к папке AppData/Roaming/ACF
    let mut appdata_path = dirs::config_dir().ok_or("Не удалось найти директорию AppData")?;
    appdata_path.push("ACF");
    
    // Создаем папку, если её ещё нет
    fs::create_dir_all(&appdata_path)?;
    let config_file_path = appdata_path.join("version.json");

    // Читаем текущую локальную версию из файла (если файл есть)
    let local_version = if config_file_path.exists() {
        let file_content = fs::read_to_string(&config_file_path)?;
        let config: AppConfig = serde_json::from_str(&file_content)?;
        config.last_known_version
    } else {
        "v0.0.0".to_string() // Значение по умолчанию для первой установки
    };

    println!("Локальная сохраненная версия: {}", local_version);

    // Запрашиваем актуальный тег с GitHub API с таймаутом
    let response = ureq::get(url)
        .set("User-Agent", "Slint-Installer-App")
        .timeout(Duration::from_secs(5))
        .call()?;
    
    let json_text = response.into_string()?;
    let json_parsed: serde_json::Value = serde_json::from_str(&json_text)?;
    
    if let Some(tag) = json_parsed.get("tag_name") {
        if let Some(tag_str) = tag.as_str() {
            let remote_version = tag_str.trim().to_string();
            println!("Актуальная версия на сервере: {}", remote_version);

            // Если версия на сервере отличается от локальной — выводим уведомление
            if remote_version != local_version {
                println!("Обнаружено новое обновление: {} -> {}", local_version, remote_version);

                // Сохраняем новую версию в JSON файл
                let new_config = AppConfig {
                    last_known_version: remote_version.clone(),
                };
                let serialized = serde_json::to_string_pretty(&new_config)?;
                fs::write(&config_file_path, serialized)?;

                // Показываем уведомление пользователю
                let summary_msg = format!("Доступно обновление {}", remote_version);
                let body_msg = format!("Старая версия: {}\nНажмите кнопку ниже, чтобы скачать обновление.", local_version);
                
                let _ = show_notification_net8(&summary_msg, &body_msg, github_release_page);
            } else {
                println!("У вас установлена самая актуальная версия.");
            }
        }
    }
    
    Ok(())
}



fn main() -> Result<(), slint::PlatformError> {
    // --- ПРОВЕРКА ОБНОВЛЕНИЙ И СРАВНЕНИЕ С JSON В APPDATA ---
    let release_api_url = "https://api.github.com/repos/ACF137/APP-ORGANIZER/releases/latest"; 
    let github_html_page = "https://github.com/ACF137/APP-ORGANIZER/releases/latest";

    match check_and_update_version_system(release_api_url, github_html_page) {
        Ok(_) => println!("Проверка обновлений успешно завершена."),
        Err(e) => eprintln!("Проверка обновлений пропущена или завершилась с ошибкой: {}", e),
    }

    // --- ПРОВЕРКА .NET 8 И АВТОМАТИЧЕСКАЯ ОФФЛАЙН-УСТАНОВКА ---
    if is_net8_installed() {
        let _ = show_notification_net8("Мастер установки", ".NET 8 Runtime обнаружен в системе.", "https://microsoft.com");
    } else {
        let _ = show_notification_net8("Мастер установки", ".NET 8 не найден. Установка из локального пакета...", "https://microsoft.com");
        if let Err(err) = auto_install_net8_offline() {
            eprintln!("Не удалось автоматически развернуть локальный .NET 8: {}", err);
        }
    }

    let main_window = MainWindow::new()?;

    // Системный коллбек: Закрытие окна
    let ui_weak = main_window.as_weak();
    main_window.on_close_clicked(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let _ = ui.hide();
        }
    });

    // Системный коллбек: Сворачивание окна
    let ui_weak = main_window.as_weak();
    main_window.on_minimize_clicked(move || {
        println!("Запрос сворачивания окна");
    });

    // Системный коллбек: Перетаскивание безрамочного окна мышкой
    let ui_weak = main_window.as_weak();
    main_window.on_move_window(move |delta_x, delta_y| {
        if let Some(ui) = ui_weak.upgrade() {
            let window = ui.window();
            let scale_factor = window.scale_factor();
            let logical_pos = window.position().to_logical(scale_factor);
            
            let new_x = logical_pos.x + delta_x;
            let new_y = logical_pos.y + delta_y;
            
            window.set_position(slint::WindowPosition::Logical(
                slint::LogicalPosition::new(new_x, new_y)
            ));
        }
    });

    // Нативный выбор папки через проводник Windows
    let ui_weak = main_window.as_weak();
    main_window.on_select_folder_clicked(move || {
        if let Some(ui) = ui_weak.upgrade() {
            if let Some(folder_path) = rfd::FileDialog::new().pick_folder() {
                let path_str = folder_path.to_string_lossy().into_owned();
                ui.set_install_path(slint::SharedString::from(path_str));
            }
        }
    });

    // Логика распаковки файлов и жесткое правило подпапки "\ACF"
    let ui_weak = main_window.as_weak();
    main_window.on_start_installation(move || {
        let ui = ui_weak.upgrade().unwrap();
        let base_path_str = ui.get_install_path().to_string();
        
        // Формируем конечный путь с принудительной подпапкой ACF
        let mut final_path = PathBuf::from(base_path_str);
        final_path.push("ACF");
        
        let target_path_str = final_path.to_string_lossy().into_owned();
        println!("Конечный путь развертывания: {}", target_path_str);

        let thread_ui_weak = ui_weak.clone();

        std::thread::spawn(move || {
            match extract_embedded_dir(&target_path_str, thread_ui_weak.clone()) {
                Ok(_) => {
                    println!("Установка из памяти успешно завершена!");
                }
                Err(err) => {
                    eprintln!("Ошибка при записи файлов: {}", err);
                    let error_msg = slint::SharedString::from(format!("Ошибка: {}", err));
                    let _ = slint::invoke_from_event_loop({
                        let ui_clone = thread_ui_weak.clone();
                        move || {
                            if let Some(ui) = ui_clone.upgrade() {
                                ui.set_current_status_text(error_msg);
                            }
                        }
                    });
                    std::thread::sleep(Duration::from_secs(3));
                }
            }

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = thread_ui_weak.upgrade() {
                    ui.set_current_step(4);
                }
            });
        });
    });

    main_window.run()
}

