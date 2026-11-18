//! `docs-media oled <dir>`: profiles.png.
//! `docs-media keymap <dir>`: one SVG per layer.
//! `docs-media assemble <frames> <dir>`: every folder of PNG frames in
//! `<frames>` becomes `<dir>/<folder>.gif` (`frame-ms` / `hold-ms` files in
//! a folder set its timing; default 50 ms a frame).

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use kb_docs_media::{assemble, img, keymap_svg, scenes};

fn write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = dir.join(name);
    fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("{:>9}  {}", bytes.len(), path.display());
    Ok(())
}

fn oled(dir: &Path) -> Result<(), String> {
    let (strip, palette) = scenes::profiles_strip();
    write(dir, "profiles.png", &img::png_indexed(&strip, palette))
}

fn keymap(dir: &Path) -> Result<(), String> {
    for (i, name) in ["default", "symbols", "nav", "wm"].iter().enumerate() {
        write(
            dir,
            &format!("keymap-{name}.svg"),
            keymap_svg::layer(i).as_bytes(),
        )?;
    }
    Ok(())
}

fn number(dir: &Path, file: &str, default: u32) -> u32 {
    fs::read_to_string(dir.join(file))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

fn assemble_all(frames: &Path, dir: &Path) -> Result<(), String> {
    let mut scenes: Vec<_> = fs::read_dir(frames)
        .map_err(|e| format!("{}: {e}", frames.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    scenes.sort();
    for scene in scenes {
        let name = scene
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("scene")
            .to_string();
        let gif = assemble::assemble(
            &scene,
            number(&scene, "frame-ms", 50),
            number(&scene, "hold-ms", 0),
        )?;
        write(dir, &format!("{name}.gif"), &gif)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["oled", dir] => fs::create_dir_all(dir)
            .map_err(|e| e.to_string())
            .and_then(|()| oled(Path::new(dir))),
        ["keymap", dir] => fs::create_dir_all(dir)
            .map_err(|e| e.to_string())
            .and_then(|()| keymap(Path::new(dir))),
        ["assemble", frames, dir] => fs::create_dir_all(dir)
            .map_err(|e| e.to_string())
            .and_then(|()| assemble_all(Path::new(frames), Path::new(dir))),
        _ => Err("usage: docs-media oled <dir> | keymap <dir> | assemble <frames> <dir>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("docs-media: {e}");
            ExitCode::FAILURE
        }
    }
}
