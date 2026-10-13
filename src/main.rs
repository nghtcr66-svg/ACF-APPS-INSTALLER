// #![windows_subsystem = "windows"]
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


// УМНАЯ СИСТЕМНАЯ ФУНКЦИЯ С РЕКУРСИВНЫМ АВТОПОИСКОМ ЛЮБОГО EXE ДЛЯ ЯРЛЫКА
fn create_installer_shortcuts(target_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let target_path = Path::new(target_dir);

    println!("Запуск глубокого рекурсивного сканирования директории: {:?}", target_path);

    let mut found_exe_path = None;

    // Рекурсивно обходим вообще ВСЕ вложенные папки (максимальная глубина 5 уровней)
    for entry in walkdir::WalkDir::new(target_path)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok()) 
    {
        let path = entry.path();
        
        // Ищем файлы с расширением .exe
        if path.is_file() && path.extension().map_or(false, |ext| ext == "exe") {
            let file_name = path.file_name().unwrap().to_string_lossy().to_lowercase();
            
            // Пропускаем служебные дампы и рантайм dotnet, если они там есть
            if !file_name.contains("dotnet") && !file_name.contains("createdump") {
                found_exe_path = Some(path.to_path_buf());
                break; // Как только нашли первый настоящий EXE — останавливаем поиск
            }
        }
    }

    // Проверяем, удалось ли что-то найти
    let exe_path = match found_exe_path {
        Some(path) => path,
        None => {
            return Err(format!(
                "Критическая ошибка: в папке установки {:?} физически нет ни одного запускаемого .exe файла!", 
                target_path
            ).into());
        }
    };

    println!("Глубокий автопоиск успешно обнаружил исполняемый файл: {:?}", exe_path);

    let exe_str = exe_path.to_string_lossy().into_owned();
    // Рабочей директорией для ярлыка делаем именно ту ПАПКУ, где физически лежит найденный EXE
    let working_dir_str = exe_path.parent().unwrap().to_string_lossy().into_owned();

    // Создаем ярлык в глобальном меню Пуск для всех пользователей
    if let Ok(program_data) = std::env::var("ProgramData") {
        let mut start_menu_path = PathBuf::from(program_data);
        start_menu_path.push("Microsoft");
        start_menu_path.push("Windows");
        start_menu_path.push("Start Menu");
        start_menu_path.push("Programs");
        start_menu_path.push("ACF"); // Подпапка в меню Пуск

        // Создаем подпапку ACF в Пуске, если её ещё нет
        fs::create_dir_all(&start_menu_path)?;

        // Имя для ярлыка автоматически берём из имени найденного файла (без расширения .exe)
        let shortcut_name = exe_path.file_stem().unwrap().to_string_lossy();
        let shortcut_start_menu = start_menu_path.join(format!("{}.lnk", shortcut_name));
        let shortcut_start_menu_str = shortcut_start_menu.to_string_lossy().into_owned();

        // Универсальный PowerShell скрипт через COM-объект WScript.Shell
        let ps_script = format!(
            "$WshShell = New-Object -ComObject WScript.Shell; \
             $Shortcut = $WshShell.CreateShortcut('{}'); \
             $Shortcut.TargetPath = '{}'; \
             $Shortcut.WorkingDirectory = '{}'; \
             $Shortcut.Save()",
            shortcut_start_menu_str, exe_str, working_dir_str
        );

        // Скрытый вызов PowerShell
        let output = Command::new("powershell")
            .args(&["-NoProfile", "-Command", &ps_script])
            .output()?;

        if output.status.success() {
            println!("Глобальный системный ярлык [{}.lnk] успешно прописан в меню Пуск!", shortcut_name);
        } else {
            eprintln!("Системная ошибка PowerShell при сохранении ярлыка: {}", String::from_utf8_lossy(&output.stderr));
        }
    } else {
        eprintln!("Не удалось прочитать системную переменную окружения %ProgramData%");
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

    std::thread::spawn(|| {
        let release_api_url = "https://api.github.com/repos/ACF137/APP-ORGANIZER/releases/latest"; 
        let github_html_page = "https://github.com/ACF137/APP-ORGANIZER/releases/latest";

    match check_and_update_version_system(release_api_url, github_html_page) {
            Ok(_) => println!("Проверка обновлений успешно завершена."),
            Err(e) => eprintln!("Проверка обновлений пропущена (ошибка сети/сессии): {}", e),
        }
    });

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

    // Логика распаковки файлов и умное правило подпапки "\ACF"
    let ui_weak = main_window.as_weak();
    main_window.on_start_installation(move || {
        let ui = ui_weak.upgrade().unwrap();
        let base_path_str = ui.get_install_path().to_string();
        
        let mut final_path = PathBuf::from(&base_path_str);
        
        // УМНАЯ ПРОВЕРКА: если путь уже заканчивается на "ACF", 
        // то мы НЕ добавляем её ещё раз, чтобы избежать матрёшки
        if !base_path_str.ends_with("ACF") && !base_path_str.ends_with("ACF\\") && !base_path_str.ends_with("ACF/") {
            final_path.push("ACF");
        }
        
        let target_path_str = final_path.to_string_lossy().into_owned();
        println!("Конечный путь развертывания: {}", target_path_str);

        // Делаем клоны строк для безопасной передачи в поток
        let path_for_extract = target_path_str.clone(); 
        let path_for_shortcuts = target_path_str.clone(); 

        let thread_ui_weak = ui_weak.clone();

        std::thread::spawn(move || {
            match extract_embedded_dir(&path_for_extract, thread_ui_weak.clone()) {
                Ok(_) => {
                    println!("Установка из памяти успешно завершена! Синхронизация диска...");
                    
                    // ФИКС: Даем Windows 200 миллисекунд, чтобы полностью сбросить кэш 
                    // файловой системы на диск и закрыть все дескрипторы файлов
                    std::thread::sleep(Duration::from_millis(200));
                    
                    // Передаем чистый, проверенный путь без дубликатов
                    match create_installer_shortcuts(&path_for_shortcuts) {
                        Ok(_) => println!("Все системные ярлыки успешно созданы."),
                        Err(err) => {
                            eprintln!("Не удалось создать ярлыки приложения: {}", err);
                        }
                    }
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

            // Переключаем визард на экран окончания установки (Шаг 4)
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = thread_ui_weak.upgrade() {
                    ui.set_current_step(4);
                }
            });
        });
    });

    // Запуск главного цикла обработки событий графического интерфейса
    main_window.run()
}


