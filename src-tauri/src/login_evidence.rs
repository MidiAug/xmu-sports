//! Local, bounded login diagnostics. Pixels never participate in navigation.
use std::io::Write;
use std::path::{Path,PathBuf};
use std::time::{Instant,SystemTime};
use serde_json::{json,Value};
use windows::core::{w,PCWSTR};
use windows::Win32::Foundation::{POINT,RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext,DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

#[link(name="ntdll")]
extern "system" {
    fn RtlGetVersion(info:*mut windows::Win32::System::SystemInformation::OSVERSIONINFOW)->i32;
}

fn windows_version()->Value {
    static VERSION:std::sync::OnceLock<Value>=std::sync::OnceLock::new();
    VERSION.get_or_init(||unsafe {
        use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
        let mut info=OSVERSIONINFOW{dwOSVersionInfoSize:std::mem::size_of::<OSVERSIONINFOW>() as u32,..Default::default()};
        if RtlGetVersion(&mut info)<0 {return Value::Null;}
        json!({"major":info.dwMajorVersion,"minor":info.dwMinorVersion,"build":info.dwBuildNumber})
    }).clone()
}

pub(crate) fn file_version(path:&Path)->Option<String> {
    use windows::Win32::Storage::FileSystem::*;
    let wide:Vec<u16>=path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    unsafe {
        let len=GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()),None);
        if len==0 || len>1024*1024 {return None;}
        // Aligned for VS_FIXEDFILEINFO.
        let mut buffer=vec![0u32;(len as usize).div_ceil(4)];
        GetFileVersionInfoW(PCWSTR(wide.as_ptr()),None,len,buffer.as_mut_ptr().cast()).ok()?;
        let mut ptr=std::ptr::null_mut();let mut size=0;
        if !VerQueryValueW(buffer.as_ptr().cast(),w!("\\"),&mut ptr,&mut size).as_bool()
            || ptr.is_null() || size<std::mem::size_of::<VS_FIXEDFILEINFO>() as u32 {return None;}
        let info=&*(ptr as *const VS_FIXEDFILEINFO);
        Some(format!("{}.{}.{}.{}",info.dwFileVersionMS>>16,info.dwFileVersionMS&65535,
            info.dwFileVersionLS>>16,info.dwFileVersionLS&65535))
    }
}

pub(crate) fn environment()->Value {
    let kernel=std::env::var_os("SystemRoot").and_then(|root|file_version(&PathBuf::from(root).join("System32/ntdll.dll")));
    let versions=crate::wechat_window::discover_wechat_candidates().unwrap_or_default().iter()
        .filter_map(|surface|file_version(Path::new(&surface.owner.app_identity))).collect::<Vec<_>>();
    json!({"windows_version":windows_version(),"windows_kernel_file_version":kernel,"wechat_appex_versions":versions,
        "app_version":env!("CARGO_PKG_VERSION"),"architecture":std::env::consts::ARCH})
}

pub(crate) struct LoginEvidence { dir:PathBuf, events:Option<std::fs::File>, start:Instant, captures:u32 }
impl LoginEvidence {
    pub(crate) fn new(root:&Path,run_id:&str)->Self {
        let parent=root.join("login-diagnostics");
        prune(&parent);
        let dir=parent.join(uuid::Uuid::new_v4().to_string());
        let events=std::fs::create_dir_all(dir.join("evidence")).ok()
            .and_then(|_|std::fs::File::create(dir.join("events.jsonl")).ok());
        log::info!(target:"http","[login_diagnostics] run_id={} diagnostic_id={} writable={}",
            run_id,dir.file_name().unwrap_or_default().to_string_lossy(),events.is_some());
        let mut evidence=Self{dir,events,start:Instant::now(),captures:0};
        evidence.record("started",json!({"run_id":run_id,"environment":environment()}));
        evidence
    }
    pub(crate) fn record(&mut self,phase:&str,details:Value) {
        if let Some(file)=self.events.as_mut() {
            let event=json!({"wall_time":chrono::Local::now().to_rfc3339(),"monotonic_ms":self.start.elapsed().as_millis(),
                "phase":phase,"window":crate::wechat_window::background_status(),"overlay":crate::login_overlay::diagnostics(),"details":details,
                "environment":if phase=="normal_ui_foreground" {environment()} else {Value::Null}});
            if writeln!(file,"{event}").and_then(|_|file.flush()).is_err() {
                self.events=None;
                log::warn!(target:"http","[login_evidence] write_failed=true");
            }
        }
    }
    pub(crate) fn snapshot(&mut self,reason:&str,overlay:&crate::login_overlay::LoginOverlay,active:&impl Fn()->bool) {
        if !active() || self.captures>=6 || self.events.is_none() {return;}
        self.captures+=1;
        let filename=format!("login-{:02}.bmp",self.captures);
        let result=(|| {
            let _hidden=overlay.suspend()?;
            // Wait for the compositor to remove our layers before reading pixels.
            unsafe {let _=windows::Win32::Graphics::Dwm::DwmFlush();}
            if !active() {return Err("CANCELLED".into());}
            capture_unique(&self.dir.join("evidence").join(&filename))
        })();
        self.record("window_capture",match result {
            Ok(quality)=>json!({"reason":reason,"file":format!("evidence/{filename}"),"quality":quality}),
            Err(code)=>json!({"reason":reason,"capture_error":code}),
        });
    }
}

fn prune(parent:&Path) {
    let Ok(canonical_parent)=parent.canonicalize() else {return;};
    let mut entries:Vec<_>=std::fs::read_dir(parent).into_iter().flatten().flatten()
        .filter_map(|entry| {
            // Only our own UUID directories; never follow junctions/symlinks.
            if !entry.file_type().ok()?.is_dir() || uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err() {return None;}
            Some((entry.metadata().ok()?.modified().ok()?,entry.path()))
        }).collect();
    entries.sort_by(|a,b|b.0.cmp(&a.0));
    for (index,(modified,path)) in entries.into_iter().enumerate() {
        if index>=19 || SystemTime::now().duration_since(modified).unwrap_or_default()>crate::diag_export::RUNS_MAX_AGE {
            if path.canonicalize().is_ok_and(|target|target.parent()==Some(canonical_parent.as_path())) {
                let _=std::fs::remove_dir_all(path);
            }
        }
    }
}

fn capture_unique(path:&Path)->Result<&'static str,String> {
    let surfaces=crate::wechat_window::discover_wechat_candidates()?;
    if surfaces.len()!=1 || !surfaces[0].visible {return Err("CAPTURE_TARGET_MISSING_OR_AMBIGUOUS".into());}
    let target=&surfaces[0];
    let bytes=capture_client(target.hwnd())?;
    let current=crate::wechat_window::discover_wechat_candidates()?;
    if current.len()!=1 || current[0].hwnd_u64!=target.hwnd_u64 || current[0].owner!=target.owner {return Err("CAPTURE_TARGET_CHANGED".into());}
    let low_info=bytes[54..].chunks_exact(4).all(|pixel|pixel[..3]==bytes[54..57]);
    std::fs::write(path,bytes).map_err(|_|"CAPTURE_WRITE_FAILED")?;
    Ok(if low_info {"low_information"} else {"captured"})
}

fn bmp(width:i32,height:i32,pixels:&[u8])->Vec<u8> {
    let mut out=b"BM".to_vec();
    out.extend_from_slice(&(54u32+pixels.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0;4]);out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&(-height).to_le_bytes());out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());out.extend_from_slice(&[0;24]);out.extend_from_slice(pixels);
    out
}

fn capture_client(hwnd:windows::Win32::Foundation::HWND)->Result<Vec<u8>,String> {
    unsafe {
        let old_dpi=SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let result=(|| {
            let mut rect=RECT::default();GetClientRect(hwnd,&mut rect).map_err(|_|"CAPTURE_BOUNDS_FAILED")?;
            let width=rect.right;let height=rect.bottom;
            if width<=0 || height<=0 || width as i64*height as i64>16_000_000 {return Err("CAPTURE_BOUNDS_INVALID".into());}
            let mut origin=POINT::default();
            if !ClientToScreen(hwnd,&mut origin).as_bool() {return Err("CAPTURE_BOUNDS_FAILED".into());}
            // Refuse occluded/minimized/offscreen windows. Never store a desktop frame.
            let unobscured=||GetForegroundWindow()==hwnd && !IsIconic(hwnd).as_bool() && [1,2,3].iter().all(|x|
                [1,2,3].iter().all(|y|GetAncestor(WindowFromPoint(POINT{x:origin.x+width*x/4,y:origin.y+height*y/4}),GA_ROOT)==hwnd));
            if !unobscured() {return Err("CAPTURE_TARGET_OCCLUDED".into());}
            // WeChat's accelerated surface can return a uniform bitmap from its
            // window DC. Read only this verified, unobscured client rectangle
            // from the compositor; never allocate/copy a full desktop image.
            let source=GetDC(None);
            if source.is_invalid() {return Err("CAPTURE_DC_FAILED".into());}
            let memory=CreateCompatibleDC(Some(source));
            if memory.is_invalid() {ReleaseDC(None,source);return Err("CAPTURE_DC_FAILED".into());}
            let mut pixels=std::ptr::null_mut();
            let info=BITMAPINFO{bmiHeader:BITMAPINFOHEADER{biSize:40,biWidth:width,biHeight:-height,
                biPlanes:1,biBitCount:32,biCompression:BI_RGB.0,..Default::default()},..Default::default()};
            let bitmap=match CreateDIBSection(Some(memory),&info,DIB_RGB_COLORS,&mut pixels,None,0) {
                Ok(bitmap)=>bitmap,Err(_)=>{let _=DeleteDC(memory);ReleaseDC(None,source);return Err("CAPTURE_BITMAP_FAILED".into());},
            };
            let previous=SelectObject(memory,bitmap.into());
            let copied=BitBlt(memory,0,0,width,height,Some(source),origin.x,origin.y,SRCCOPY).is_ok();
            let _=GdiFlush();
            let mut after=RECT::default();let mut after_origin=POINT::default();
            let unchanged=GetClientRect(hwnd,&mut after).is_ok() && ClientToScreen(hwnd,&mut after_origin).as_bool()
                && after.right==width && after.bottom==height && after_origin.x==origin.x && after_origin.y==origin.y;
            let image=if copied && unchanged && unobscured() {
                Some(bmp(width,height,std::slice::from_raw_parts(pixels as *const u8,(width*height*4) as usize)))
            } else {None};
            SelectObject(memory,previous);let _=DeleteObject(bitmap.into());let _=DeleteDC(memory);ReleaseDC(None,source);
            image.ok_or_else(||"CAPTURE_COPY_FAILED_OR_OCCLUDED".into())
        })();
        if !old_dpi.0.is_null() {let _=SetThreadDpiAwarenessContext(old_dpi);}
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bitmap_retains_dimensions_orientation_and_pixels() {
        let pixels=[0u8,0,255,0,0,255,0,0];
        let image=bmp(2,1,&pixels);
        assert_eq!(&image[..2],b"BM");assert_eq!(image.len(),62);
        assert_eq!(i32::from_le_bytes(image[22..26].try_into().unwrap()),-1);
        assert_eq!(&image[54..],&pixels);
    }
    #[test]
    fn environment_has_os_build_without_paths_or_usernames() {
        let env=environment();assert!(env["windows_version"]["build"].as_u64().is_some());
        assert!(!env.to_string().contains("Users"));
    }
    #[test]
    fn retention_is_bounded_and_preserves_unowned_directories() {
        let parent=std::env::temp_dir().join(format!("xmu-login-retention-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(parent.join("unowned")).unwrap();
        for _ in 0..22 {std::fs::create_dir(parent.join(uuid::Uuid::new_v4().to_string())).unwrap();}
        prune(&parent);
        assert!(parent.join("unowned").is_dir());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(),20);
        // The exact temp target is created and owned by this test.
        std::fs::remove_dir_all(parent).unwrap();
    }
}
