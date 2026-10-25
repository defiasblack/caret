//! The small, stable PDFium C surface used by the disposable visual helper.
//! No PDFium handle, native allocation, or parser runs in the editor process.
use crate::visual_viewer::{RenderRequest, Transform};
use std::{
    ffi::{c_char, c_int, c_ulong, c_void, CStr},
    io,
    path::{Path, PathBuf},
};

type Handle = *mut c_void;
struct Library(Handle);
impl Library {
    fn open(path: &Path) -> io::Result<Self> {
        #[cfg(windows)]
        let handle = {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::System::LibraryLoader::*;
            let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            // Only an explicit path is loaded; never search the current directory.
            unsafe {
                LoadLibraryExW(
                    path.as_ptr(),
                    std::ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
                )
            }
        };
        #[cfg(unix)]
        let handle = {
            use std::os::unix::ffi::OsStrExt;
            let path =
                std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
            unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) }
        };
        if handle.is_null() {
            Err(io::Error::other(format!("Cannot load PDFium at {}. Run scripts/setup_pdfium.py, or set CARET_PDFIUM_LIBRARY to the full library path", path.display())))
        } else {
            Ok(Self(handle))
        }
    }
    // SAFETY: callers supply the exact signatures from PDFium's public/fpdfview.h.
    unsafe fn symbol<T: Copy>(&self, name: &CStr) -> io::Result<T> {
        #[cfg(windows)]
        let symbol =
            windows_sys::Win32::System::LibraryLoader::GetProcAddress(self.0, name.as_ptr().cast())
                .map(|function| function as *const ())
                .unwrap_or(std::ptr::null());
        #[cfg(unix)]
        let symbol = libc::dlsym(self.0, name.as_ptr()).cast_const();
        if symbol.is_null() {
            return Err(io::Error::other(format!(
                "PDFium is missing {}",
                name.to_string_lossy()
            )));
        }
        assert_eq!(std::mem::size_of::<T>(), std::mem::size_of_val(&symbol));
        Ok(std::mem::transmute_copy(&symbol))
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::FreeLibrary(self.0);
        }
        #[cfg(unix)]
        unsafe {
            libc::dlclose(self.0);
        }
    }
}

macro_rules! bindings {
    ($($field:ident: $signature:ty = $name:literal),* $(,)?) => {
        struct Pdfium { $($field: $signature,)* _library: Library }
        impl Pdfium {
            fn load() -> io::Result<Self> {
                let library = Library::open(&library_path()?)?;
                let api = Self {
                    // SAFETY: these stable exports have the signatures in fpdfview.h.
                    $($field: unsafe { library.symbol(CStr::from_bytes_with_nul(concat!($name, "\0").as_bytes()).unwrap())? },)*
                    _library: library,
                };
                unsafe { (api.init)(); }
                Ok(api)
            }
        }
    }
}
bindings! {
    init: unsafe extern "system" fn() = "FPDF_InitLibrary",
    destroy: unsafe extern "system" fn() = "FPDF_DestroyLibrary",
    load: unsafe extern "system" fn(*const c_void, usize, *const c_char) -> Handle = "FPDF_LoadMemDocument64",
    close_document: unsafe extern "system" fn(Handle) = "FPDF_CloseDocument",
    count: unsafe extern "system" fn(Handle) -> c_int = "FPDF_GetPageCount",
    page: unsafe extern "system" fn(Handle, c_int) -> Handle = "FPDF_LoadPage",
    close_page: unsafe extern "system" fn(Handle) = "FPDF_ClosePage",
    width: unsafe extern "system" fn(Handle) -> f32 = "FPDF_GetPageWidthF",
    height: unsafe extern "system" fn(Handle) -> f32 = "FPDF_GetPageHeightF",
    bitmap: unsafe extern "system" fn(c_int, c_int, c_int, *mut c_void, c_int) -> Handle = "FPDFBitmap_CreateEx",
    close_bitmap: unsafe extern "system" fn(Handle) = "FPDFBitmap_Destroy",
    render: unsafe extern "system" fn(Handle, Handle, c_int, c_int, c_int, c_int, c_int, c_int) = "FPDF_RenderPageBitmap",
    error: unsafe extern "system" fn() -> c_ulong = "FPDF_GetLastError",
}
impl Drop for Pdfium {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)();
        }
    }
}
struct Resource {
    handle: Handle,
    close: unsafe extern "system" fn(Handle),
}
impl Drop for Resource {
    fn drop(&mut self) {
        unsafe {
            (self.close)(self.handle);
        }
    }
}

fn library_path() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("CARET_PDFIUM_LIBRARY") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(io::Error::other(
                "CARET_PDFIUM_LIBRARY must be an absolute path",
            ));
        }
        return Ok(path);
    }
    let name = if cfg!(windows) {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    };
    let sibling = std::env::current_exe()?.with_file_name(name);
    if sibling.is_file() {
        return Ok(sibling);
    }
    // Source builds can use the setup script without copying libraries after each build.
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("runtime/pdfium")
        .join(if cfg!(windows) { "bin" } else { "lib" })
        .join(name))
}

pub fn render(
    bytes: &[u8],
    request: &RenderRequest,
) -> io::Result<(image::RgbImage, usize, Transform)> {
    let api = Pdfium::load()?;
    // SAFETY: all resources below are nested inside the library and input bytes'
    // lifetime. The bitmap borrows our checked, contiguous BGRA allocation.
    unsafe {
        let handle = (api.load)(bytes.as_ptr().cast(), bytes.len(), std::ptr::null());
        if handle.is_null() {
            return Err(io::Error::other(match (api.error)() {
                4 => "Password-protected PDFs are not supported",
                _ => "PDFium could not read this PDF (invalid or damaged document)",
            }));
        }
        let document = Resource {
            handle,
            close: api.close_document,
        };
        let count = (api.count)(document.handle).max(0) as usize;
        if count == 0 {
            return Err(io::Error::other("PDF contains no pages"));
        }
        let handle = (api.page)(document.handle, request.page.min(count - 1) as c_int);
        if handle.is_null() {
            return Err(io::Error::other("PDF page could not be loaded"));
        }
        let page = Resource {
            handle,
            close: api.close_page,
        };
        let transform = Transform::new(
            (api.width)(page.handle) as f64,
            (api.height)(page.handle) as f64,
            request,
        )?;
        let mut pixels = vec![255u8; request.width as usize * request.height as usize * 4];
        let handle = (api.bitmap)(
            request.width as c_int,
            request.height as c_int,
            4,
            pixels.as_mut_ptr().cast(),
            request.width as c_int * 4,
        );
        if handle.is_null() {
            return Err(io::Error::other("PDF bitmap allocation failed"));
        }
        let bitmap = Resource {
            handle,
            close: api.close_bitmap,
        };
        (api.render)(
            bitmap.handle,
            page.handle,
            transform.left,
            transform.top,
            transform.width as c_int,
            transform.height as c_int,
            0,
            1,
        );
        drop(bitmap);
        let mut rgb = image::RgbImage::new(request.width, request.height);
        for (source, target) in pixels.as_chunks::<4>().0.iter().zip(rgb.pixels_mut()) {
            *target = image::Rgb([source[2], source[1], source[0]]);
        }
        Ok((rgb, count, transform))
    }
}
