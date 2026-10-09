#[path = "../src/test_support.rs"]
mod test_support;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn load(path: &Path, output: &Path) -> Option<Result<serde_json::Value, String>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_caret"))
        .arg("--office-worker")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(fs::File::create(output).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if child.try_wait().unwrap().is_some() {
            child.wait().unwrap();
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Office helper exceeded deadline");
        }
        thread::sleep(Duration::from_millis(5));
    }
    serde_json::from_slice(&fs::read(output).unwrap()).ok()
}
#[test]
fn helper_rejects_oversized_malformed_and_sparse_allocation_bombs() {
    let root = std::env::temp_dir().join(format!("caret-helper-tests-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let file = root.join("book.xlsx");
    let output = root.join("response.json");
    let large = fs::File::create(&file).unwrap();
    large.set_len(32 * 1024 * 1024 + 1).unwrap();
    drop(large);
    assert!(load(&file, &output)
        .unwrap()
        .unwrap_err()
        .contains("32 MiB"));
    fs::write(&file, b"malformed archive").unwrap();
    assert!(load(&file, &output).unwrap().is_err());
    // Calamine would allocate a dense 1,048,576 by 16,384 range. The helper's
    // allocator must reject that allocation before it reaches the OS.
    test_support::write_workbook(
        &file,
        r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="XFD1048576"><v>2</v></c></row>"#,
    );
    assert!(load(&file, &output).is_none_or(|result| result.is_err()));
    test_support::write_workbook(&file, r#"<row r="3"><c r="B3"><v>7</v></c></row>"#);
    let viewer = load(&file, &output).unwrap().unwrap();
    assert_eq!(
        viewer["content"]["Spreadsheet"]["sheets"][0]["cells"]["2"]["1"],
        "7"
    );
    fs::remove_dir_all(root).unwrap();
}
