//! Exercise actual native decoding in a disposable, memory-limited process.
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn render(path: &Path, request: &Value, output: &Path) -> Result<Value, String> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_caret"));
    #[cfg(target_os = "macos")]
    command
        .env("MallocNanoZone", "0")
        .env("MallocSpaceEfficient", "1");
    let mut child = command
        .args(["--visual-worker"])
        .arg(path)
        .arg(request.to_string())
        .stdin(Stdio::null())
        .stdout(fs::File::create(output).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if child.try_wait().unwrap().is_some() {
            child.wait().unwrap();
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Visual helper did not terminate");
        }
        thread::sleep(Duration::from_millis(5));
    }
    let bytes = fs::read(output).unwrap();
    assert!(bytes.len() <= 64 * 1024 * 1024);
    serde_json::from_slice::<Result<Value, String>>(&bytes)
        .unwrap_or_else(|_| Err("Memory-limited renderer stopped".into()))
}

fn request() -> Value {
    json!({"page":0,"width":120,"height":160,"columns":15,"rows":10,"zoom":100,"pan_x":0,"pan_y":0,"fit_width":false,"protocol":"Sixel"})
}
#[test]
fn pdf_pages_render_with_layout_and_image_budgets_are_enforced() {
    let root = std::env::temp_dir().join(format!("caret-visual-helper-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let output = root.join("response.json");
    let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/visual-two-pages.pdf");
    let first = render(&pdf, &request(), &output);
    // A source build can run ordinary tests without installing optional native
    // PDFium. CI installs it, so any other renderer failure must fail the suite.
    if first
        .as_ref()
        .is_err_and(|error| error.contains("Cannot load PDFium"))
    {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must install PDFium: {first:?}"
        );
        eprintln!("PDF assertions skipped: run python scripts/setup_pdfium.py");
    } else {
        let first = first.unwrap();
        assert_eq!(first["pages"], 2);
        assert_eq!(first["transform"]["source_width"], 300);
        assert_eq!(first["transform"]["source_height"], 400);
        assert!(first["lines"][0].as_str().unwrap().starts_with("\x1bP"));
        // This fixture contains a blue rectangle, page text, and white margins.
        let mut second_request = request();
        second_request["page"] = json!(1);
        let second = render(&pdf, &second_request, &output).unwrap();
        assert_eq!(second["page"], 1);
        assert_eq!(second["transform"]["source_width"], 400);
        assert_ne!(first["lines"], second["lines"]);
        second_request["page"] = json!(999);
        assert_eq!(render(&pdf, &second_request, &output).unwrap()["page"], 1);
    }

    let image = root.join("picture.png");
    image::RgbImage::from_pixel(4, 2, image::Rgb([20, 100, 200]))
        .save(&image)
        .unwrap();
    let rendered = render(&image, &request(), &output).unwrap();
    assert_eq!(rendered["transform"]["source_width"], 4);
    for extension in ["jpg", "bmp", "ico", "tif", "ppm", "tga"] {
        let path = root.join(format!("picture.{extension}"));
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            2,
            image::Rgba([20, 100, 200, 255]),
        ))
        .save(&path)
        .unwrap();
        let rendered = render(&path, &request(), &output)
            .unwrap_or_else(|error| panic!("{extension}: {error}"));
        assert_eq!(rendered["transform"]["source_width"], 4);
        assert_eq!(rendered["transform"]["source_height"], 2);
    }
    let mut zoom = request();
    zoom["zoom"] = json!(800);
    zoom["pan_x"] = json!(32768);
    let rendered = render(&image, &zoom, &output).unwrap();
    assert_eq!(rendered["transform"]["pan_x"], 840);
    for protocol in ["Kitty", "Blocks"] {
        let mut request = request();
        request["protocol"] = json!(protocol);
        let frame = render(&image, &request, &output).unwrap();
        assert!(!frame["lines"].as_array().unwrap().is_empty());
    }
    fs::write(&image, b"not an image").unwrap();
    assert!(render(&image, &request(), &output).is_err());
    let oversized = fs::File::create(&image).unwrap();
    oversized.set_len(32 * 1024 * 1024 + 1).unwrap();
    drop(oversized);
    assert!(render(&image, &request(), &output)
        .unwrap_err()
        .contains("32 MiB"));
    let bmp = root.join("bomb.bmp");
    let mut header = vec![0u8; 54];
    header[..2].copy_from_slice(b"BM");
    header[10..14].copy_from_slice(&54u32.to_le_bytes());
    header[14..18].copy_from_slice(&40u32.to_le_bytes());
    header[18..22].copy_from_slice(&32000u32.to_le_bytes());
    header[22..26].copy_from_slice(&32000u32.to_le_bytes());
    header[26..28].copy_from_slice(&1u16.to_le_bytes());
    header[28..30].copy_from_slice(&24u16.to_le_bytes());
    fs::write(&bmp, header).unwrap();
    assert!(render(&bmp, &request(), &output)
        .unwrap_err()
        .contains("pixel"));
    fs::remove_dir_all(root).unwrap();
}
