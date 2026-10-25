//! Bounded, cancellable PDF/image rasterization and terminal graphics.
use crossterm::event::{KeyCode, KeyEvent};
use image::{GenericImageView, ImageDecoder};
use serde::{Deserialize, Serialize};
use std::{
    fmt::Write as _,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

pub const WORKER_ARGUMENT: &str = "--visual-worker";
const INPUT_BYTES: u64 = 32 * 1024 * 1024;
const MEMORY_BYTES: usize = 512 * 1024 * 1024;
const OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const SOURCE_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_WIDTH: u32 = 2048;
const MAX_HEIGHT: u32 = 1536;
const TIMEOUT: Duration = Duration::from_secs(5);
pub const KITTY_DELETE: &str = "\x1b_Ga=d,d=I,i=34762143,q=2\x1b\\";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Sixel,
    Kitty,
    Blocks,
}
impl Protocol {
    pub fn detect() -> Self {
        let forced = std::env::var("CARET_GRAPHICS").unwrap_or_default();
        Self::choose(
            &forced,
            &std::env::var("TERM").unwrap_or_default(),
            &std::env::var("TERM_PROGRAM").unwrap_or_default(),
            std::env::var_os("WT_SESSION").is_some(),
        )
    }
    fn choose(forced: &str, term: &str, program: &str, windows_terminal: bool) -> Self {
        match forced {
            "sixel" => Self::Sixel,
            "kitty" => Self::Kitty,
            "blocks" => Self::Blocks,
            _ if term.contains("kitty") || program == "ghostty" => Self::Kitty,
            _ if windows_terminal
                || term.contains("sixel")
                || matches!(program, "WezTerm" | "mintty" | "contour") =>
            {
                Self::Sixel
            }
            _ => Self::Blocks,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Sixel => "Sixel",
            Self::Kitty => "Kitty",
            Self::Blocks => "color cells",
        }
    }
}

pub fn supports(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "pdf"
            | "png"
            | "jpg"
            | "jpeg"
            | "bmp"
            | "ico"
            | "tif"
            | "tiff"
            | "pnm"
            | "ppm"
            | "pgm"
            | "pbm"
            | "tga"
    )
}
pub fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderRequest {
    #[serde(default)]
    pub generation: u64,
    pub page: usize,
    pub width: u32,
    pub height: u32,
    pub columns: u16,
    pub rows: u16,
    pub zoom: u32,
    pub pan_x: u32,
    pub pan_y: u32,
    pub fit_width: bool,
    pub protocol: Protocol,
}
impl RenderRequest {
    fn validate(self) -> io::Result<()> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_WIDTH
            || self.height > MAX_HEIGHT
            || self.columns == 0
            || self.rows == 0
            || !(25..=800).contains(&self.zoom)
            || self.pan_x > 32768
            || self.pan_y > 32768
        {
            return Err(io::Error::other(
                "Visual viewport exceeds the rendering budget",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Transform {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub pan_x: u32,
    pub pan_y: u32,
    pub source_width: u32,
    pub source_height: u32,
}
impl Transform {
    pub fn new(width: f64, height: f64, request: &RenderRequest) -> io::Result<Self> {
        request.validate()?;
        if !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
            || width > 1_000_000.0
            || height > 1_000_000.0
        {
            return Err(io::Error::other(
                "Invalid or oversized page/image dimensions",
            ));
        }
        let scale = if request.fit_width {
            request.width as f64 / width
        } else {
            (request.width as f64 / width).min(request.height as f64 / height)
        } * request.zoom as f64
            / 100.0;
        let scaled_width = (width * scale).round().max(1.0);
        let scaled_height = (height * scale).round().max(1.0);
        if scaled_width > 32768.0 || scaled_height > 32768.0 {
            return Err(io::Error::other(
                "Zoomed page exceeds the 32768-pixel dimension budget; use Fit page",
            ));
        }
        let (scaled_width, scaled_height) = (scaled_width as u32, scaled_height as u32);
        let pan_x = request
            .pan_x
            .min(scaled_width.saturating_sub(request.width));
        let pan_y = request
            .pan_y
            .min(scaled_height.saturating_sub(request.height));
        Ok(Self {
            width: scaled_width,
            height: scaled_height,
            left: (request.width.saturating_sub(scaled_width) / 2) as i32 - pan_x as i32,
            top: (request.height.saturating_sub(scaled_height) / 2) as i32 - pan_y as i32,
            pan_x,
            pan_y,
            source_width: width.round() as u32,
            source_height: height.round() as u32,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Frame {
    pub page: usize,
    pub pages: usize,
    pub transform: Transform,
    pub lines: Vec<String>,
}

pub struct VisualViewer {
    pub path: PathBuf,
    pub protocol: Protocol,
    pub frame: Option<Frame>,
    pub error: Option<String>,
    pub loading: bool,
    pages: usize,
    request: RenderRequest,
    submitted: Option<RenderRequest>,
}
impl VisualViewer {
    pub fn new(path: &Path) -> Self {
        let protocol = Protocol::detect();
        Self {
            path: path.to_path_buf(),
            protocol,
            frame: None,
            error: None,
            loading: false,
            pages: 0,
            request: RenderRequest {
                generation: 0,
                page: 0,
                width: 1,
                height: 1,
                columns: 1,
                rows: 1,
                zoom: 100,
                pan_x: 0,
                pan_y: 0,
                fit_width: false,
                protocol,
            },
            submitted: None,
        }
    }
    pub fn label(&self) -> String {
        let page = if is_pdf(&self.path) {
            format!(
                "PDF · Page {}/{}",
                self.request.page + 1,
                if self.pages == 0 {
                    "…".into()
                } else {
                    self.pages.to_string()
                }
            )
        } else {
            "IMAGE".into()
        };
        format!(
            "{page} · {}% {} · {}",
            self.request.zoom,
            if self.request.fit_width {
                "width"
            } else {
                "fit"
            },
            self.protocol.label()
        )
    }
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let pages = self.pages.max(1);
        let old = self.request;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return true,
            KeyCode::Char('+' | '=') => self.request.zoom = (self.request.zoom * 5 / 4).min(800),
            KeyCode::Char('-' | '_') => self.request.zoom = (self.request.zoom * 4 / 5).max(25),
            KeyCode::Char('f' | '0') => {
                self.request.zoom = 100;
                self.request.fit_width = false;
                self.reset_pan();
            }
            KeyCode::Char('w') => {
                self.request.zoom = 100;
                self.request.fit_width = true;
                self.reset_pan();
            }
            KeyCode::PageDown | KeyCode::Char('n' | ' ') => {
                self.request.page = (self.request.page + 1).min(pages.saturating_sub(1))
            }
            KeyCode::PageUp | KeyCode::Char('p') => {
                self.request.page = self.request.page.saturating_sub(1)
            }
            KeyCode::Home => self.request.page = 0,
            KeyCode::End => self.request.page = pages.saturating_sub(1),
            KeyCode::Left | KeyCode::Char('h') => {
                self.request.pan_x = self
                    .request
                    .pan_x
                    .saturating_sub((self.request.width / 8).max(1))
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.request.pan_x = self
                    .request
                    .pan_x
                    .saturating_add((self.request.width / 8).max(1))
                    .min(32768)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.request.pan_y = self
                    .request
                    .pan_y
                    .saturating_sub((self.request.height / 8).max(1))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.request.pan_y = self
                    .request
                    .pan_y
                    .saturating_add((self.request.height / 8).max(1))
                    .min(32768)
            }
            KeyCode::Char('r') => self.request.generation = self.request.generation.wrapping_add(1),
            _ => {}
        }
        if self.request.page != old.page {
            self.reset_pan();
        }
        // Clamp immediately, so repeated panning at an edge does not launch work.
        if let Some(frame) = &self.frame {
            if old.page == self.request.page
                && old.zoom == self.request.zoom
                && old.fit_width == self.request.fit_width
            {
                self.request.pan_x = self
                    .request
                    .pan_x
                    .min(frame.transform.width.saturating_sub(self.request.width));
                self.request.pan_y = self
                    .request
                    .pan_y
                    .min(frame.transform.height.saturating_sub(self.request.height));
            }
        }
        if old != self.request {
            self.request.generation = old.generation.wrapping_add(1);
        }
        false
    }
    fn reset_pan(&mut self) {
        self.request.pan_x = 0;
        self.request.pan_y = 0;
    }
    pub fn prepare(
        &mut self,
        columns: usize,
        rows: usize,
        cell: (u16, u16),
        loader: &mut VisualLoader,
    ) {
        let old = self.request;
        self.request.columns = columns.clamp(1, u16::MAX as usize) as u16;
        self.request.rows = rows.clamp(1, u16::MAX as usize) as u16;
        let (cw, ch) = if self.protocol == Protocol::Blocks {
            (1, 2)
        } else {
            cell
        };
        self.request.width = (columns as u32)
            .saturating_mul(cw as u32)
            .clamp(1, MAX_WIDTH);
        self.request.height = (rows as u32).saturating_mul(ch as u32).clamp(1, MAX_HEIGHT);
        if old != self.request {
            self.request.generation = old.generation.wrapping_add(1);
        }
        if self.submitted == Some(self.request) {
            return;
        }
        self.submitted = Some(self.request);
        self.error = None;
        self.frame = None;
        self.loading = true;
        if let Err(error) = loader.start(&self.path, self.request) {
            self.accept(self.request, Err(error));
        }
    }
    pub fn accept(&mut self, request: RenderRequest, result: io::Result<Frame>) -> bool {
        if request != self.request {
            return false;
        }
        self.loading = false;
        match result {
            Ok(frame) => {
                self.pages = frame.pages;
                self.request.page = frame.page;
                self.request.pan_x = frame.transform.pan_x;
                self.request.pan_y = frame.transform.pan_y;
                self.submitted = Some(self.request);
                self.frame = Some(frame);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        true
    }
}

/// Pixel dimensions are supplied by the terminal on Unix. Windows and SSH can
/// override the conservative 8x16 fallback for their font/DPI through this env var.
pub fn cell_size() -> (u16, u16) {
    if let Ok(value) = std::env::var("CARET_CELL_PIXELS") {
        if let Some((width, height)) = value
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<u16>().ok()?, h.parse::<u16>().ok()?)))
        {
            if (1..=64).contains(&width) && (1..=128).contains(&height) {
                return (width, height);
            }
        }
    }
    if let Ok(size) = crossterm::terminal::window_size() {
        if size.columns > 0
            && size.rows > 0
            && size.width >= size.columns
            && size.height >= size.rows
        {
            return (size.width / size.columns, size.height / size.rows);
        }
    }
    (8, 16)
}

struct Job {
    child: Child,
    result: Receiver<io::Result<Frame>>,
    started: Instant,
    request: RenderRequest,
}
impl Drop for Job {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Default)]
pub struct VisualLoader {
    job: Option<Job>,
}
impl VisualLoader {
    pub fn cancel(&mut self) {
        self.job = None;
    }
    pub fn start(&mut self, path: &Path, request: RenderRequest) -> io::Result<()> {
        self.cancel();
        request.validate()?;
        let started = Instant::now();
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg(WORKER_ARGUMENT)
            .arg(path)
            .arg(serde_json::to_string(&request)?)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        let output = child.stdout.take().expect("piped stdout");
        let (sender, result) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = sender.send(read_response(output));
        });
        self.job = Some(Job {
            child,
            result,
            started,
            request,
        });
        Ok(())
    }
    pub fn poll(&mut self) -> Option<(RenderRequest, io::Result<Frame>)> {
        let job = self.job.as_mut()?;
        let request = job.request;
        if job.started.elapsed() >= TIMEOUT {
            self.cancel();
            return Some((
                request,
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Visual loading exceeded the five-second budget; use a smaller image or PDF",
                )),
            ));
        }
        if let Ok(result) = job.result.try_recv() {
            self.job = None;
            return Some((request, result));
        }
        None
    }
}
fn read_response(output: impl Read) -> io::Result<Frame> {
    let mut bytes = Vec::new();
    output.take(OUTPUT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > OUTPUT_BYTES {
        return Err(io::Error::other(
            "Visual response exceeds the 64 MiB budget",
        ));
    }
    let response: Result<Frame, String> = serde_json::from_slice(&bytes).map_err(|_| {
        io::Error::other(
            "Visual renderer stopped: malformed content or the 512 MiB memory budget was exceeded",
        )
    })?;
    response.map_err(io::Error::other)
}

pub fn worker() -> i32 {
    crate::ALLOCATOR.set_limit(MEMORY_BYTES);
    let response = (|| -> io::Result<Frame> {
        let _memory = limit_native_memory()?;
        let path = std::env::args_os()
            .nth(2)
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("Missing visual path"))?;
        let request: RenderRequest = serde_json::from_str(
            &std::env::args_os()
                .nth(3)
                .ok_or_else(|| io::Error::other("Missing visual request"))?
                .into_string()
                .map_err(|_| io::Error::other("Invalid visual request"))?,
        )?;
        render_file(&path, request)
    })()
    .map_err(|error| error.to_string());
    let bytes = match serde_json::to_vec(&response) {
        Ok(bytes) if bytes.len() as u64 <= OUTPUT_BYTES => bytes,
        _ => return 1,
    };
    if io::stdout().lock().write_all(&bytes).is_ok() {
        0
    } else {
        1
    }
}

#[cfg(windows)]
fn limit_native_memory() -> io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::{JobObjects::*, Threading::GetCurrentProcess};
    // Enforce the native heap limit before loading PDFium or opening any input.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let owned = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.ProcessMemoryLimit = MEMORY_BYTES;
    if unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
        || unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(owned)
}
#[cfg(unix)]
fn limit_native_memory() -> io::Result<()> {
    let limits = libc::rlimit {
        rlim_cur: MEMORY_BYTES as libc::rlim_t,
        rlim_max: MEMORY_BYTES as libc::rlim_t,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limits) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn render_file(path: &Path, request: RenderRequest) -> io::Result<Frame> {
    request.validate()?;
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Visual viewer requires a regular file"));
    }
    if file.metadata()?.len() > INPUT_BYTES {
        return Err(io::Error::other(
            "Visual file exceeds the 32 MiB input budget",
        ));
    }
    let mut bytes = Vec::new();
    file.take(INPUT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > INPUT_BYTES {
        return Err(io::Error::other(
            "Visual file exceeds the 32 MiB input budget",
        ));
    }
    let (pixels, pages, transform) = if is_pdf(path) {
        crate::pdf::render(&bytes, &request)?
    } else {
        render_image(bytes, &request)?
    };
    let lines = match request.protocol {
        Protocol::Sixel => vec![sixel(&pixels)],
        Protocol::Kitty => vec![kitty(&pixels, request.columns, request.rows)],
        Protocol::Blocks => blocks(&pixels),
    };
    Ok(Frame {
        page: request.page.min(pages - 1),
        pages,
        transform,
        lines,
    })
}

fn render_image(
    bytes: Vec<u8>,
    request: &RenderRequest,
) -> io::Result<(image::RgbImage, usize, Transform)> {
    let mut reader = image::ImageReader::new(io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(io::Error::other)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > SOURCE_PIXELS {
        return Err(io::Error::other(
            "Image exceeds the 32-million-pixel decode budget",
        ));
    }
    let orientation = decoder.orientation().map_err(io::Error::other)?;
    let mut source = image::DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
    source.apply_orientation(orientation);
    let transform = Transform::new(source.width() as f64, source.height() as f64, request)?;
    let mut output = image::RgbImage::new(request.width, request.height);
    // Sample only the visible viewport; zoom never allocates an enlarged image.
    for (x, y, pixel) in output.enumerate_pixels_mut() {
        let sx = x as i32 - transform.left;
        let sy = y as i32 - transform.top;
        if sx < 0 || sy < 0 || sx >= transform.width as i32 || sy >= transform.height as i32 {
            *pixel = image::Rgb([30, 30, 30]);
            continue;
        }
        let fx = (sx as f64 + 0.5) * source.width() as f64 / transform.width as f64 - 0.5;
        let fy = (sy as f64 + 0.5) * source.height() as f64 / transform.height as f64 - 0.5;
        let x0 = fx.max(0.0).floor() as u32;
        let y0 = fy.max(0.0).floor() as u32;
        let x1 = (x0 + 1).min(source.width() - 1);
        let y1 = (y0 + 1).min(source.height() - 1);
        let (tx, ty) = (fx.max(0.0).fract(), fy.max(0.0).fract());
        let corners = [
            source.get_pixel(x0, y0),
            source.get_pixel(x1, y0),
            source.get_pixel(x0, y1),
            source.get_pixel(x1, y1),
        ];
        let weights = [
            (1.0 - tx) * (1.0 - ty),
            tx * (1.0 - ty),
            (1.0 - tx) * ty,
            tx * ty,
        ];
        let checker = if ((x / 12) + (y / 12)) % 2 == 0 {
            224.0
        } else {
            192.0
        };
        for channel in 0..3 {
            let mut color = 0.0;
            for (corner, weight) in corners.iter().zip(weights) {
                let alpha = corner[3] as f64 / 255.0;
                color += weight * (corner[channel] as f64 * alpha + checker * (1.0 - alpha));
            }
            pixel[channel] = color.round() as u8;
        }
    }
    Ok((output, 1, transform))
}

fn sixel(image: &image::RgbImage) -> String {
    let mut output = format!("\x1bP0;1;0q\"1;1;{};{}", image.width(), image.height());
    for index in 0..256u16 {
        let _ = write!(
            output,
            "#{index};2;{};{};{}",
            (index >> 5) * 100 / 7,
            ((index >> 2) & 7) * 100 / 7,
            (index & 3) * 100 / 3
        );
    }
    let width = image.width() as usize;
    let mut planes = vec![vec![0u8; width]; 256];
    for top in (0..image.height()).step_by(6) {
        for plane in &mut planes {
            plane.fill(0);
        }
        for y in top..(top + 6).min(image.height()) {
            for x in 0..image.width() {
                let pixel = image.get_pixel(x, y);
                let color = (pixel[0] & 0xe0) | ((pixel[1] >> 3) & 0x1c) | (pixel[2] >> 6);
                planes[color as usize][x as usize] |= 1 << (y - top);
            }
        }
        for (color, plane) in planes.iter().enumerate() {
            let Some(last) = plane.iter().rposition(|value| *value != 0) else {
                continue;
            };
            let _ = write!(output, "#{color}");
            let mut start = 0;
            while start <= last {
                let value = plane[start];
                let mut end = start + 1;
                while end <= last && plane[end] == value {
                    end += 1;
                }
                let count = end - start;
                if count > 3 {
                    let _ = write!(output, "!{count}{}", (value + 63) as char);
                } else {
                    for _ in 0..count {
                        output.push((value + 63) as char);
                    }
                }
                start = end;
            }
            output.push('$');
        }
        if top + 6 < image.height() {
            output.push('-');
        }
    }
    output.push_str("\x1b\\");
    output
}
fn kitty(image: &image::RgbImage, columns: u16, rows: u16) -> String {
    let payload = crate::clipboard::base64(image.as_raw());
    let mut output = String::new();
    let chunks = payload.as_bytes().chunks(4096);
    let count = chunks.len();
    for (index, chunk) in chunks.enumerate() {
        let more = u8::from(index + 1 < count);
        if index == 0 {
            let _ = write!(
                output,
                "\x1b_Ga=T,f=24,s={},v={},c={columns},r={rows},i=34762143,q=2,C=1,m={more};",
                image.width(),
                image.height()
            );
        } else {
            let _ = write!(output, "\x1b_Gm={more};");
        }
        output.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        output.push_str("\x1b\\");
    }
    output
}
fn blocks(image: &image::RgbImage) -> Vec<String> {
    (0..image.height())
        .step_by(2)
        .map(|y| {
            let mut line = String::new();
            for x in 0..image.width() {
                let top = image.get_pixel(x, y);
                let bottom = image.get_pixel(x, (y + 1).min(image.height() - 1));
                let _ = write!(
                    line,
                    "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                    top[0], top[1], top[2], bottom[0], bottom[1], bottom[2]
                );
            }
            line.push_str("\x1b[0m");
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> RenderRequest {
        RenderRequest {
            generation: 0,
            page: 0,
            width: 100,
            height: 80,
            columns: 10,
            rows: 5,
            zoom: 100,
            pan_x: 0,
            pan_y: 0,
            fit_width: false,
            protocol: Protocol::Sixel,
        }
    }
    #[test]
    fn fits_preserve_aspect_and_pan_is_clamped() {
        let mut request = request();
        let fit = Transform::new(200.0, 100.0, &request).unwrap();
        assert_eq!((fit.width, fit.height, fit.left, fit.top), (100, 50, 0, 15));
        request.zoom = 400;
        request.pan_x = 32768;
        request.pan_y = 32768;
        let zoomed = Transform::new(200.0, 100.0, &request).unwrap();
        assert_eq!(
            (zoomed.pan_x, zoomed.pan_y, zoomed.left, zoomed.top),
            (300, 120, -300, -120)
        );
        assert!(Transform::new(f64::NAN, 100.0, &request).is_err());
        request.width = MAX_WIDTH + 1;
        assert!(Transform::new(100.0, 100.0, &request).is_err());
    }
    #[test]
    fn protocol_selection_is_explicit_and_has_a_portable_fallback() {
        assert_eq!(
            Protocol::choose("", "xterm-256color", "", true),
            Protocol::Sixel
        );
        assert_eq!(
            Protocol::choose("", "xterm-kitty", "", false),
            Protocol::Kitty
        );
        assert_eq!(
            Protocol::choose("blocks", "xterm-kitty", "", true),
            Protocol::Blocks
        );
        assert_eq!(Protocol::choose("", "dumb", "", false), Protocol::Blocks);
        assert!(supports(Path::new("PHOTO.JPEG")));
        assert!(supports(Path::new("book.PDF")));
        assert!(!supports(Path::new("main.rs")));
    }
    #[test]
    fn encoders_produce_bounded_graphics_and_correct_palette() {
        let image = image::RgbImage::from_pixel(8, 7, image::Rgb([255, 0, 0]));
        let sixel = sixel(&image);
        assert!(sixel.starts_with("\x1bP0;1;0q\"1;1;8;7"));
        assert!(sixel.contains("#224;2;100;0;0"));
        assert!(sixel.contains("#224!8~$-#224!8@$"));
        assert!(sixel.ends_with("\x1b\\"));
        let kitty = kitty(&image, 8, 4);
        assert!(kitty.contains("s=8,v=7,c=8,r=4,i=34762143,q=2,C=1,m=0;"));
        assert_eq!(blocks(&image).len(), 4);
    }
    #[test]
    fn image_decode_composites_alpha_and_obeys_dimensions() {
        let source = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 0]));
        let mut bytes = io::Cursor::new(Vec::new());
        source
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let (image, pages, transform) = render_image(bytes.into_inner(), &request()).unwrap();
        assert_eq!(pages, 1);
        assert_eq!((transform.width, transform.height), (80, 80));
        assert_eq!(image.get_pixel(10, 0).0, [224, 224, 224]);
        assert_eq!(image.get_pixel(0, 0).0, [30, 30, 30]);
        assert!(render_image(b"broken".to_vec(), &request()).is_err());
    }
    #[test]
    fn navigation_restores_fit_and_rejects_stale_work_on_close() {
        let mut viewer = VisualViewer::new(Path::new("book.pdf"));
        viewer.pages = 3;
        viewer.frame = Some(Frame {
            page: 0,
            pages: 3,
            transform: Transform::new(100.0, 100.0, &request()).unwrap(),
            lines: vec![],
        });
        for code in [KeyCode::PageDown, KeyCode::Char('+'), KeyCode::Char('w')] {
            assert!(!viewer.handle_key(KeyEvent::from(code)));
        }
        assert_eq!(viewer.request.page, 1);
        assert!(viewer.request.fit_width);
        viewer.handle_key(KeyEvent::from(KeyCode::Char('f')));
        assert_eq!(viewer.request.zoom, 100);
        assert!(!viewer.request.fit_width);
        assert!(viewer.handle_key(KeyEvent::from(KeyCode::Esc)));
    }
    #[test]
    fn malformed_and_oversized_responses_fail_cleanly() {
        assert!(read_response(&b"not json"[..]).is_err());
        assert!(read_response(io::repeat(b'x').take(OUTPUT_BYTES + 1))
            .unwrap_err()
            .to_string()
            .contains("64 MiB"));
    }

    #[test]
    fn stale_render_cannot_revert_navigation_or_reload() {
        let mut viewer = VisualViewer::new(Path::new("book.pdf"));
        viewer.pages = 4;
        let old = viewer.request;
        viewer.handle_key(KeyEvent::from(KeyCode::PageDown));
        viewer.handle_key(KeyEvent::from(KeyCode::PageDown));
        assert_eq!(viewer.request.page, 2);
        assert!(!viewer.accept(old, Err(io::Error::other("obsolete failure"))));
        assert!(viewer.error.is_none());
        assert_eq!(viewer.request.page, 2);
        let before_reload = viewer.request;
        viewer.handle_key(KeyEvent::from(KeyCode::Char('r')));
        assert!(!viewer.accept(before_reload, Err(io::Error::other("old reload"))));
    }

    #[test]
    fn cancellation_and_expired_jobs_are_reaped() {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ]);
            command
        };
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "exec sleep 30"]);
            command
        };
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (_sender, result) = mpsc::sync_channel(1);
        let mut loader = VisualLoader {
            job: Some(Job {
                child,
                result,
                started: Instant::now() - TIMEOUT,
                request: request(),
            }),
        };
        let (issued, result) = loader.poll().unwrap();
        assert_eq!(issued, request());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(loader.job.is_none());
        loader.cancel();
        assert!(loader.poll().is_none());
    }
}
