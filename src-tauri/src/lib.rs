#![deny(unsafe_code)]

pub mod attachments;
pub mod conversation;
pub mod error;
pub mod indexer;
pub mod json_stream;
pub mod models;
pub mod safe_root;
pub mod server;
pub mod store;
pub mod structure_inspector;

use std::{io, path::PathBuf};

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub fn run_desktop() {
    tauri::Builder::default()
        .setup(|app| {
            let web_root = resolve_web_root(app)?;
            let (listener, state, bound) =
                server::bind_desktop_loopback(web_root.clone(), app.handle().clone())
                    .map_err(|_| io::Error::other("local server setup failed"))?;
            server::spawn_loopback(listener, state, web_root, bound.shutdown.clone());

            let launch_url = format!("{}/#token={}", bound.origin, bound.token)
                .parse()
                .map_err(|_| io::Error::other("local launch URL failed"))?;
            let allowed_origin = bound.origin.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(launch_url))
                .title("ChatGPT History Browser")
                .inner_size(1280.0, 820.0)
                .min_inner_size(900.0, 620.0)
                .resizable(true)
                .incognito(true)
                .on_navigation(move |url| {
                    url.as_str()
                        .strip_prefix(&allowed_origin)
                        .is_some_and(|suffix| {
                            suffix.is_empty() || suffix.starts_with(['/', '#'])
                        })
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("application runtime failed");
}

fn resolve_web_root<R: tauri::Runtime>(app: &tauri::App<R>) -> tauri::Result<PathBuf> {
    if cfg!(debug_assertions)
        && let Ok(current) = std::env::current_dir()
    {
        for candidate in [current.join("dist"), current.join("../dist")] {
            if candidate.join("index.html").is_file() {
                return Ok(candidate);
            }
        }
    }
    app.path()
        .resolve("web", tauri::path::BaseDirectory::Resource)
}
