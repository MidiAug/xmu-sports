//! Public release downloads are isolated from the authenticated booking worker.
use reqwest::{
    blocking::{Client, Response},
    Url,
};
use serde::{Deserialize,Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, atomic::{AtomicBool,Ordering}},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tokio::sync::watch;

const REPO: &str = "https://github.com/MidiAug/xmu-sports";
const LATEST: &str = "https://api.github.com/repos/MidiAug/xmu-sports/releases/latest";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
#[derive(Clone, Debug, Serialize)]
pub struct ReleaseInfo {
    status: String,
    version: String,
    notes: String,
    size: u64,
}
#[derive(Clone, Debug)]
struct Candidate {
    info: ReleaseInfo,
    name: String,
    url: String,
    sha256: String,
}
#[derive(Deserialize,Serialize)]
struct Downloaded {
    path: PathBuf,
    size: u64,
    sha256: String,
}
#[derive(Default)]
struct UpdateState {
    cached: Option<(Instant, Result<ReleaseInfo, String>)>,
    candidate: Option<Candidate>,
    downloaded: Option<Downloaded>,
}
static STATE: OnceLock<Mutex<UpdateState>> = OnceLock::new();
static UPDATE_AVAILABLE:AtomicBool=AtomicBool::new(false);
#[derive(Default)]
struct Downloads {
    active:Option<(String,watch::Sender<bool>)>,
    cancelled_ids:std::collections::VecDeque<String>,
}
impl Downloads {
    fn begin(&mut self,id:&str)->Result<watch::Receiver<bool>,String> {
        if uuid::Uuid::parse_str(id).is_err() {return Err("下载标识无效".into());}
        if let Some(index)=self.cancelled_ids.iter().position(|cancelled|cancelled==id) {
            self.cancelled_ids.remove(index);return Err("UPDATE_CANCELLED".into());
        }
        if self.active.is_some() {return Err("正在下载更新，请稍候".into());}
        let (sender,receiver)=watch::channel(false);self.active=Some((id.into(),sender));Ok(receiver)
    }
    fn cancel(&mut self,id:&str)->bool {
        if let Some((active,sender))=&self.active {
            if active==id {sender.send_replace(true);return true;}
        }
        // Handles IPC scheduling where cancellation arrives before registration.
        if !self.cancelled_ids.iter().any(|cancelled|cancelled==id) {
            self.cancelled_ids.push_back(id.into());
            if self.cancelled_ids.len()>32 {self.cancelled_ids.pop_front();}
        }
        false
    }
}
static DOWNLOADS:OnceLock<Mutex<Downloads>>=OnceLock::new();
fn downloads()->&'static Mutex<Downloads> {DOWNLOADS.get_or_init(Default::default)}
fn downloading()->bool {downloads().lock().map(|d|d.active.is_some()).unwrap_or(true)}
struct DownloadLease(String);
impl Drop for DownloadLease {
    fn drop(&mut self) {if let Ok(mut d)=downloads().lock() {
        if d.active.as_ref().is_some_and(|(id,_)|id==&self.0) {d.active=None;}
    }}
}
#[tauri::command]
pub fn cancel_app_update_download(download_id:String)->Result<bool,String> {
    if uuid::Uuid::parse_str(&download_id).is_err() {return Err("下载标识无效".into());}
    Ok(downloads().lock().map_err(|_|"下载状态不可用")?.cancel(&download_id))
}

async fn interruptible<T>(future:impl std::future::Future<Output=Result<T,reqwest::Error>>,
    cancel:&mut watch::Receiver<bool>,ready:&impl Fn()->Result<(),String>)->Result<T,String> {
    tokio::pin!(future);
    loop {
        if *cancel.borrow() {return Err("UPDATE_CANCELLED".into());}
        ready()?;
        tokio::select! {
            biased;
            _=cancel.changed()=>return Err("UPDATE_CANCELLED".into()),
            result=&mut future=>return result.map_err(|_|"下载连接中断，请重试".into()),
            _=tokio::time::sleep(Duration::from_millis(100))=>{},
        }
    }
}

struct DownloadDirectory {path:PathBuf,keep:bool}
impl Drop for DownloadDirectory {
    fn drop(&mut self) {if !self.keep {let _=fs::remove_dir_all(&self.path);}}
}

async fn stream_async(mut response:reqwest::Response,dest:&Path,size:u64,hash:&str,
    cancel:&mut watch::Receiver<bool>,ready:&impl Fn()->Result<(),String>,mut progress:impl FnMut(u64))->Result<(),String> {
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(dest).map_err(|_|"无法创建下载文件")?;
    let mut digest=Sha256::new();let mut count=0u64;
    while let Some(chunk)=interruptible(response.chunk(),cancel,ready).await? {
        count+=chunk.len() as u64;
        if count>size || count>MAX_PACKAGE {return Err("安装包大小与发布信息不一致".into());}
        file.write_all(&chunk).map_err(|_|"保存安装包失败，请检查磁盘空间")?;
        digest.update(&chunk);progress(count);
    }
    if *cancel.borrow() {return Err("UPDATE_CANCELLED".into());}ready()?;
    if count!=size || format!("{:x}",digest.finalize())!=hash {return Err("安装包校验失败，已删除，请重试".into());}
    file.sync_all().map_err(|_|"保存安装包失败")?;Ok(())
}
pub(crate) fn has_update()->bool {UPDATE_AVAILABLE.load(Ordering::Relaxed)}
fn state() -> &'static Mutex<UpdateState> {
    STATE.get_or_init(Default::default)
}

fn version(value: &str) -> Result<[u64; 3], String> {
    let values = value
        .strip_prefix('v')
        .unwrap_or(value)
        .split('.')
        .map(|v| {
            if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Err("发布版本格式无效".into());
            }
            v.parse().map_err(|_| "发布版本格式无效".to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    values.try_into().map_err(|_| "发布版本格式无效".into())
}
fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.fragment().is_none()
        && match url.host_str() {
            Some("api.github.com") => url.as_str() == LATEST,
            Some("github.com") => url
                .path()
                .starts_with("/MidiAug/xmu-sports/releases/download/"),
            Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com") => true,
            _ => false,
        }
}
fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent("xmu-sports-updater")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法启动更新服务".into())
}
fn open(client: &Client, raw: &str, seconds: u64) -> Result<Response, String> {
    open_route(client, raw, seconds, None)
}
fn package_mirror(raw: &str, mirror: Option<&str>) -> Result<Option<Url>, String> {
    let prefix = match mirror {
        None | Some("official") => return Ok(None),
        Some("gh-proxy") => "https://gh-proxy.com/",
        Some("ghproxy") => "https://ghproxy.net/",
        _ => return Err("请选择内置下载线路".into()),
    };
    let source = Url::parse(raw).map_err(|_| "更新地址无效")?;
    if !allowed(&source) || source.host_str() != Some("github.com")
        || source.query().is_some() || !source.path().ends_with("_x64-setup.exe") {
        return Err("镜像仅用于官方下载的安装包".into());
    }
    Url::parse(&format!("{prefix}{raw}")).map(Some).map_err(|_| "下载线路无效".into())
}
fn download_allowed(url: &Url, mirror: Option<&Url>) -> bool {
    allowed(url) || mirror == Some(url)
}
fn open_route(client: &Client, raw: &str, seconds: u64, mirror: Option<&Url>) -> Result<Response, String> {
    let mut url = match mirror { Some(url) => url.clone(), None => Url::parse(raw).map_err(|_| "更新地址无效")? };
    for _ in 0..6 {
        if !download_allowed(&url, mirror) {
            return Err("更新地址不受信任，请查看发布页".into());
        }
        let response = client
            .get(url.clone())
            .timeout(Duration::from_secs(seconds))
            .send()
            .map_err(|_| if mirror.is_some() { "无法连接所选下载线路，请更换线路后重试" } else { "无法连接 GitHub，请检查网络后重试" }.to_string())?;
        let code = response.status().as_u16();
        if code == 200 {
            return Ok(response);
        }
        if [301, 302, 303, 307, 308].contains(&code) && raw != LATEST {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("更新跳转无效")?;
            url = url.join(location).map_err(|_| "更新跳转无效")?;
            continue;
        }
        return Err(match code {
            404 if mirror.is_some() => "所选下载线路暂未同步安装包，请更换线路重试",
            403 | 429 if mirror.is_some() => "所选下载线路请求受限，请更换线路重试",
            404 => "尚无正式发布或发布文件不完整",
            403 | 429 => "GitHub 请求受限，请稍后重试或查看发布页",
            _ => "更新服务暂不可用，请稍后重试",
        }
        .into());
    }
    Err("更新跳转次数过多，请查看发布页".into())
}
fn bounded(mut response: Response, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    (&mut response)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取更新信息失败")?;
    if bytes.len() as u64 > limit {
        return Err("更新信息过大".into());
    }
    Ok(bytes)
}
fn asset<'a>(release: &'a Value, name: &str, tag: &str, max: u64) -> Result<&'a Value, String> {
    let matches = release["assets"]
        .as_array()
        .ok_or("缺少发布文件")?
        .iter()
        .filter(|a| a["name"].as_str() == Some(name))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err("缺少唯一的安装包或校验清单，请查看发布页".into());
    }
    let a = matches[0];
    let expected = format!(
        "{REPO}/releases/download/{tag}/{}",
        name.replace(' ', "%20")
    );
    if a["state"] != "uploaded"
        || a["size"].as_u64().filter(|n| *n > 0 && *n <= max).is_none()
        || a["browser_download_url"].as_str() != Some(expected.as_str())
    {
        return Err("发布文件信息无效，请查看发布页".into());
    }
    Ok(a)
}
fn parse_release(
    release: &Value,
    current: &str,
) -> Result<(ReleaseInfo, Option<Candidate>), String> {
    if release["draft"] != false || release["prerelease"] != false {
        return Err("暂不可用的正式发布".into());
    }
    let tag = release["tag_name"]
        .as_str()
        .filter(|v| v.starts_with('v'))
        .ok_or("发布版本格式无效")?;
    let newer = version(tag)? > version(current)?;
    let info = ReleaseInfo {
        status: if newer { "available" } else { "current" }.into(),
        version: tag.trim_start_matches('v').into(),
        notes: release["body"]
            .as_str()
            .unwrap_or("此版本未提供更新说明")
            .chars()
            .take(16000)
            .collect(),
        size: 0,
    };
    if !newer {
        return Ok((info, None));
    }
    // The package workflow builds exactly this Windows x64 NSIS asset.
    let name = format!("XMU.Sports_{}_x64-setup.exe", info.version);
    let package = asset(release, &name, tag, MAX_PACKAGE)?;
    asset(release, "SHA256SUMS.txt", tag, 65536)?;
    let info = ReleaseInfo {
        size: package["size"].as_u64().unwrap(),
        ..info
    };
    Ok((
        info.clone(),
        Some(Candidate {
            info,
            name,
            url: package["browser_download_url"].as_str().unwrap().into(),
            sha256: String::new(),
        }),
    ))
}
fn checksum(text: &str, name: &str) -> Result<String, String> {
    let mut found = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        if line.len() < 66 || !line.is_char_boundary(64) {
            continue;
        }
        let hash = &line[..64];
        if line[64..].trim_start().trim_start_matches('*') == name
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            found.push(hash.to_ascii_lowercase());
        }
    }
    if found.len() != 1 {
        return Err("安装包校验信息无效，请查看发布页".into());
    }
    Ok(found.remove(0))
}
fn check() -> Result<ReleaseInfo, String> {
    let downloads=downloads().lock().map_err(|_|"下载状态不可用")?;
    if downloads.active.is_some() {return Err("正在下载更新，请稍候".into());}
    let mut state = state()
        .try_lock()
        .map_err(|_| "正在检查或下载更新，请稍候")?;
    drop(downloads);
    if let Some((at, result)) = &state.cached {
        if at.elapsed() < Duration::from_secs(if result.is_ok() { 60 } else { 15 }) {
            return result.clone();
        }
    }
    state.candidate = None;
    let result = (|| {
        let client = client()?;
        // A 404 on this public endpoint means no stable Release has been published.
        let response = client
            .get(LATEST)
            .timeout(Duration::from_secs(30))
            .send()
            .map_err(|_| "无法连接 GitHub，请检查网络后重试")?;
        if response.status().as_u16() == 404 {
            return Ok(ReleaseInfo {
                status: "unpublished".into(),
                version: String::new(),
                notes: String::new(),
                size: 0,
            });
        }
        if !response.status().is_success() {
            return Err(if [403, 429].contains(&response.status().as_u16()) {
                "GitHub 请求受限，请稍后重试或查看发布页"
            } else {
                "更新服务暂不可用，请稍后重试"
            }
            .into());
        }
        let release: Value = serde_json::from_slice(&bounded(response, 1024 * 1024)?)
            .map_err(|_| "发布信息格式无效")?;
        let (info, candidate) = parse_release(&release, env!("CARGO_PKG_VERSION"))?;
        if let Some(mut candidate) = candidate {
            let url = format!(
                "{REPO}/releases/download/v{}/SHA256SUMS.txt",
                candidate.info.version
            );
            let bytes = bounded(open(&client, &url, 30)?, 65536)?;
            candidate.sha256 = checksum(
                std::str::from_utf8(&bytes).map_err(|_| "校验清单格式无效")?,
                &candidate.name,
            )?;
            if let Some(digest) = release["assets"]
                .as_array()
                .and_then(|a| a.iter().find(|a| a["name"] == candidate.name))
                .and_then(|a| a["digest"].as_str())
            {
                if digest != format!("sha256:{}", candidate.sha256) {
                    return Err("发布文件与校验清单不一致".into());
                }
            }
            state.candidate = Some(candidate);
        }
        Ok(info)
    })();
    if let Ok(info)=&result {UPDATE_AVAILABLE.store(info.status=="available",Ordering::Relaxed);}
    state.cached = Some((Instant::now(), result.clone()));
    result
}

#[cfg(test)]
fn stream_package(
    source: impl Read,
    dest: &Path,
    size: u64,
    hash: &str,
    progress: impl FnMut(u64),
) -> Result<(), String> {
    stream_package_checked(source,dest,size,hash,||Ok(()),progress)
}
#[cfg(test)]
fn stream_package_checked(
    mut source:impl Read,dest:&Path,size:u64,hash:&str,
    mut ready:impl FnMut()->Result<(),String>,mut progress:impl FnMut(u64),
) -> Result<(),String> {
    ready()?;
    // A failed create_new must never delete a pre-existing file.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|_| "无法创建下载文件")?;
    let result = (|| {
        let mut digest = Sha256::new();
        let mut count = 0;
        let mut buffer = [0u8; 65536];
        loop {
            ready()?;
            let len = source.read(&mut buffer).map_err(|_| "下载中断，请重试")?;
            ready()?;
            if len == 0 {
                break;
            }
            count += len as u64;
            if count > size || count > MAX_PACKAGE {
                return Err("安装包大小与发布信息不一致".into());
            }
            file.write_all(&buffer[..len])
                .map_err(|_| "保存安装包失败，请检查磁盘空间")?;
            digest.update(&buffer[..len]);
            progress(count);
        }
        if count != size || format!("{:x}", digest.finalize()) != hash {
            return Err("安装包校验失败，已删除，请重试".into());
        }
        file.sync_all().map_err(|_| "保存安装包失败")?;
        Ok(())
    })();
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(dest);
    }
    result
}
#[tauri::command]
pub async fn check_app_update() -> Result<ReleaseInfo, String> {
    tauri::async_runtime::spawn_blocking(check)
        .await
        .map_err(|_| "更新检查中断")?
}
fn cached_download(saved:&Path,root:&Path,candidate:&Candidate)->Option<Downloaded> {
    let download:Downloaded=serde_json::from_slice(&fs::read(saved).ok()?).ok()?;
    if download.size!=candidate.info.size || download.sha256!=candidate.sha256
        || download.path.file_name().is_none_or(|name|name!=candidate.name.as_str())
        || download.path.parent().and_then(Path::parent)!=Some(root)
        || verify_downloaded(&download).is_err() {return None;}
    Some(download)
}
#[tauri::command]
pub async fn download_app_update(app: tauri::AppHandle, mirror: Option<String>, automatic: Option<bool>, download_id:String) -> Result<String, String> {
        let mut cancel=downloads().lock().map_err(|_|"下载状态不可用")?.begin(&download_id)?;
        let _lease=DownloadLease(download_id.clone());
        let candidate = state()
            .try_lock()
            .map_err(|_| "正在检查或下载更新，请稍候")?.candidate.clone().ok_or("请先检查更新")?;
        let mirror = package_mirror(&candidate.url, mirror.as_deref())?;
        let root = app
            .path()
            .download_dir()
            .map_err(|_| "无法访问下载文件夹")?
            .join("XMU Sports Updates");
        // Avoid reading the arm file and taking runtime locks for every 64 KiB chunk.
        let last_ready=Mutex::new(Instant::now()-Duration::from_secs(1));
        let ready=|| {
            // Cancellation is checked by interruptible even when automatic=false.
            if automatic!=Some(true) {return Ok(());}
            let mut last_ready=last_ready.lock().map_err(|_|"更新状态不可用")?;
            if last_ready.elapsed()<Duration::from_millis(100) {return Ok(());}
            *last_ready=Instant::now();
            if automatic==Some(true) && (crate::busy_now(&app) || crate::availability::recovering() || crate::my_orders::mutating()
                || app.state::<crate::AppState>().stopping.load(Ordering::SeqCst)) {
                Err("UPDATE_PAUSED: 预约进行中，空闲后继续下载".into())
            } else {Ok(())}
        };
        ready()?;
        let saved=crate::data_root().join("updates").join("downloaded.json");
        if let Some(download)=cached_download(&saved,&root,&candidate) {
            let mut downloads=downloads().lock().map_err(|_|"下载状态不可用")?;
            if *cancel.borrow() {return Err("UPDATE_CANCELLED".into());}
            let path=download.path.to_string_lossy().into_owned();
            state().lock().map_err(|_|"更新状态不可用")?.downloaded=Some(download);downloads.active=None;return Ok(path);
        }
        let folder=root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&folder).map_err(|_| "无法创建下载文件夹")?;
        let mut directory=DownloadDirectory{path:folder.clone(),keep:false};
        let part = folder.join("download.part");
        let final_path = folder.join(&candidate.name);
        let client=reqwest::Client::builder().user_agent("xmu-sports-updater")
            .connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(600))
            .redirect(reqwest::redirect::Policy::none()).build().map_err(|_|"无法启动更新服务")?;
        let mut url=mirror.clone().unwrap_or(Url::parse(&candidate.url).map_err(|_|"更新地址无效")?);
        let mut response=None;
        for _ in 0..6 {
            if !download_allowed(&url,mirror.as_ref()) {return Err("更新地址不受信任，请查看发布页".into());}
            let current=interruptible(client.get(url.clone()).send(),&mut cancel,&ready).await?;
            let code=current.status().as_u16();
            if code==200 {response=Some(current);break;}
            if [301,302,303,307,308].contains(&code) {
                let location=current.headers().get("location").and_then(|v|v.to_str().ok()).ok_or("更新跳转无效")?;
                url=url.join(location).map_err(|_|"更新跳转无效")?;continue;
            }
            return Err(match code {403|429=>"下载线路请求受限，请更换线路后重试",404=>"所选线路尚未同步安装包，请更换线路重试",_=>"更新服务暂不可用，请稍后重试"}.into());
        }
        let response=response.ok_or("更新跳转次数过多，请查看发布页")?;
        let mut last = Instant::now() - Duration::from_secs(1);
        stream_async(
            response,
            &part,
            candidate.info.size,
            &candidate.sha256,
            &mut cancel,&ready,
            |received| {
                if last.elapsed() >= Duration::from_millis(150) || received == candidate.info.size {
                    let _ = app.emit(
                        "app-update-progress",
                        serde_json::json!({"download_id":download_id,"received":received,"total":candidate.info.size}),
                    );
                    last = Instant::now();
                }
            },
        ).await?;
        // Serialize completion with cancellation, so exactly one wins.
        let mut downloads=downloads().lock().map_err(|_|"下载状态不可用")?;
        if *cancel.borrow() {return Err("UPDATE_CANCELLED".into());}ready()?;
        if let Err(_) = fs::rename(&part, &final_path) {
            let _ = fs::remove_file(&part);
            return Err("无法保存校验后的安装包".into());
        }
        let mut state=state().try_lock().map_err(|_|"更新状态不可用")?;
        state.downloaded = Some(Downloaded {
            path: final_path.clone(),
            size: candidate.info.size,
            sha256: candidate.sha256.clone(),
        });
        if let Some(download)=&state.downloaded {
            if let Ok(bytes)=serde_json::to_vec(download) {let _=crate::atomic_fs::write_atomic(&saved,bytes);}
        }
        directory.keep=true;downloads.active=None;
        Ok(final_path.to_string_lossy().into_owned())
}
#[tauri::command]
pub fn open_update_folder(app: tauri::AppHandle) -> Result<(), String> {
    let state = state()
        .try_lock()
        .map_err(|_| "正在检查或下载更新，请稍候")?;
    let folder = state
        .downloaded
        .as_ref()
        .and_then(|p| p.path.parent())
        .map(Path::to_path_buf)
        .unwrap_or(
            app.path()
                .download_dir()
                .map_err(|_| "无法访问下载文件夹")?
                .join("XMU Sports Updates"),
        );
    fs::create_dir_all(&folder).map_err(|_| "无法创建下载文件夹")?;
    crate::diag_export::open_dir(&folder)
}

fn ensure_install_idle(
    armed: bool,
    running: bool,
    stopping: bool,
    recovering: bool,
) -> Result<(), String> {
    if armed || running || stopping || recovering {
        return Err("请先结束预约、取消定时任务或完成登录，再安装更新".into());
    }
    Ok(())
}
fn verify_downloaded(download: &Downloaded) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options
        .open(&download.path)
        .map_err(|_| "安装包不可用，请重新下载")?;
    if file.metadata().map_err(|_| "无法读取安装包")?.len() != download.size
        || download.size > MAX_PACKAGE
    {
        return Err("安装包已改变，请重新下载".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = file.read(&mut buffer).map_err(|_| "无法读取安装包")?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    if format!("{:x}", digest.finalize()) != download.sha256 {
        return Err("安装包校验失败，请重新下载".into());
    }
    Ok(file)
}
#[tauri::command]
pub async fn install_app_update(app: tauri::AppHandle) -> Result<(), String> {
    if downloading() {return Err("正在下载更新，请稍候".into());}
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = app.state::<crate::AppState>();
        let _gate = runtime.task_gate.lock().map_err(|_| "任务状态不可用")?;
        ensure_install_idle(
            crate::load_arm().armed,
            crate::child_running(&runtime),
            runtime.stopping.load(std::sync::atomic::Ordering::SeqCst),
            crate::availability::recovering() || crate::my_orders::mutating(),
        )?;
        let downloads=downloads().lock().map_err(|_|"下载状态不可用")?;
        if downloads.active.is_some() {return Err("正在下载更新，请稍候".into());}
        let state = state()
            .try_lock()
            .map_err(|_| "正在检查或下载更新，请稍候")?;
        let download = state.downloaded.as_ref().ok_or("UPDATE_PACKAGE_INVALID: 请先下载更新")?;
        // Hold a read-only sharing handle until process creation prevents replacement
        // between revalidation and launch on Windows. Never accepts frontend paths.
        let _file = verify_downloaded(download).map_err(|message| format!("UPDATE_PACKAGE_INVALID: {message}"))?;
        std::process::Command::new(&download.path)
            .arg("/UPDATE")
            .spawn()
            .map_err(|_| "无法启动安装器，请重试")?;
        drop(downloads);
        app.exit(0);
        Ok(())
    })
    .await
    .map_err(|_| "安装启动中断")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_before_registration_and_stale_cancels_do_not_cancel_next_download() {
        let mut control=Downloads::default();let old=uuid::Uuid::new_v4().to_string();let new=uuid::Uuid::new_v4().to_string();
        assert!(!control.cancel(&old));assert_eq!(control.begin(&old).unwrap_err(),"UPDATE_CANCELLED");
        let receiver=control.begin(&new).unwrap();assert!(!*receiver.borrow());
        assert!(!control.cancel(&old));assert!(!*receiver.borrow());
        assert!(control.cancel(&new));assert!(*receiver.borrow());
        control.active=None;assert!(!control.cancel(&new));
        let next=uuid::Uuid::new_v4().to_string();control.cancel(&next);control.cancel(&old);
        assert_eq!(control.begin(&next).unwrap_err(),"UPDATE_CANCELLED","late old cancel must not overwrite an early new cancel");
    }

    // A real loopback socket stalls in headers or mid-body; cancellation must
    // drop the network future instead of waiting for the 600-second timeout.
    #[test]
    fn stalled_http_is_interruptible_and_partial_directory_is_removed() {
        use std::net::TcpListener;
        for (headers_only,pause) in [(true,false),(false,false),(false,true)] {
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
            let (connected_tx,connected_rx)=std::sync::mpsc::channel();let (done_tx,done_rx)=std::sync::mpsc::channel();
            let body_tx=connected_tx.clone();
            let server=std::thread::spawn(move|| {
                let (mut socket,_)=listener.accept().unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut request=[0;2048];let _=socket.read(&mut request).unwrap();
                if !headers_only {socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabc").unwrap();}
                if headers_only {connected_tx.send(()).unwrap();}let _=done_rx.recv_timeout(Duration::from_secs(3));
            });
            let (sender,mut receiver)=watch::channel(false);
            let signal=sender.clone();let paused=std::sync::Arc::new(AtomicBool::new(false));let pause_signal=paused.clone();
            let cancel_thread=std::thread::spawn(move|| {
                connected_rx.recv_timeout(Duration::from_secs(3)).unwrap();std::thread::sleep(Duration::from_millis(100));
                if pause {pause_signal.store(true,Ordering::SeqCst);} else {signal.send_replace(true);}
            });
            let root=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());fs::create_dir_all(&root).unwrap();
            let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let result=runtime.block_on(async {
                let _directory=DownloadDirectory{path:root.clone(),keep:false};
                let client=reqwest::Client::new();
                let operation=async {
                    let ready=||if paused.load(Ordering::SeqCst) {Err("UPDATE_PAUSED".into())} else {Ok(())};
                    let response=interruptible(client.get(format!("http://{address}")).send(),&mut receiver,&ready).await?;
                    let mut received_body=false;
                    stream_async(response,&root.join("download.part"),6,&format!("{:x}",Sha256::digest(b"abcdef")),&mut receiver,&ready,|_|{
                        if !headers_only && !received_body {received_body=true;body_tx.send(()).unwrap();}
                    }).await
                };
                tokio::time::timeout(Duration::from_secs(2),operation).await.expect("cancellation must interrupt a stalled socket")
            });
            assert_eq!(result.unwrap_err(),if pause {"UPDATE_PAUSED"} else {"UPDATE_CANCELLED"});assert!(!root.exists());
            let _=done_tx.send(());cancel_thread.join().unwrap();server.join().unwrap();drop(sender);
        }
    }
    #[test]
    fn async_transfer_requires_exact_size_and_hash_before_preserving_the_package() {
        use std::net::TcpListener;
        for (payload,size,valid) in [(b"abc".as_slice(),3,true),(b"abc".as_slice(),4,false),(b"abcd".as_slice(),3,false),(b"abd".as_slice(),3,false)] {
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
            let bytes=payload.to_vec();
            let server=std::thread::spawn(move|| {
                let (mut socket,_)=listener.accept().unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut request=[0;2048];socket.read(&mut request).unwrap();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();socket.write_all(&bytes).unwrap();
            });
            let root=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());fs::create_dir_all(&root).unwrap();
            let (_sender,mut receiver)=watch::channel(false);
            let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let result=runtime.block_on(async {
                let mut directory=DownloadDirectory{path:root.clone(),keep:false};
                let response=reqwest::Client::new().get(format!("http://{address}")).timeout(Duration::from_secs(2)).send().await.unwrap();
                stream_async(response,&root.join("download.part"),size,&format!("{:x}",Sha256::digest(b"abc")),&mut receiver,&||Ok(()),|_|{}).await?;
                fs::rename(root.join("download.part"),root.join("setup.exe")).unwrap();directory.keep=true;Ok::<(),String>(())
            });
            assert_eq!(result.is_ok(),valid);
            if valid {assert_eq!(fs::read(root.join("setup.exe")).unwrap(),b"abc");fs::remove_dir_all(&root).unwrap();}
            else {assert!(!root.exists(),"invalid bytes must not remain installable");}
            server.join().unwrap();
        }
    }
    #[test]
    fn install_requires_idle_and_unchanged_download() {
        assert!(ensure_install_idle(false, false, false, false).is_ok());
        for flags in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            assert!(ensure_install_idle(flags.0, flags.1, flags.2, flags.3).is_err());
        }
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::write(&path, b"verified").unwrap();
        let download = Downloaded {
            path: path.clone(),
            size: 8,
            sha256: format!("{:x}", Sha256::digest(b"verified")),
        };
        let locked = verify_downloaded(&download).unwrap();
        #[cfg(windows)]
        assert!(fs::write(&path, b"tampered").is_err());
        drop(locked);
        fs::write(&path, b"tampered").unwrap();
        assert!(verify_downloaded(&download).is_err());
        fs::remove_file(&path).unwrap();
        assert!(verify_downloaded(&download).is_err());
    }
    fn release() -> Value {
        let mut result = serde_json::json!({"draft":false,"prerelease":false,"tag_name":"v0.2.0","body":"notes","assets":[]});
        for name in ["XMU.Sports_0.2.0_x64-setup.exe", "SHA256SUMS.txt"] {
            result["assets"].as_array_mut().unwrap().push(serde_json::json!({"name":name,"size":3,"state":"uploaded","browser_download_url":format!("{REPO}/releases/download/v0.2.0/{}",name.replace(' ',"%20"))}));
        }
        result
    }
    #[test]
    fn version_and_release_contract() {
        assert!(version("0.1.10").unwrap() > version("0.1.9").unwrap());
        for invalid in ["1.0", "1.0.0-beta", "1.0.0/evil", "1.0.-1"] {
            assert!(version(invalid).is_err());
        }
        assert_eq!(
            parse_release(&release(), "0.1.25").unwrap().0.status,
            "available"
        );
        assert!(parse_release(&release(), "0.2.0").unwrap().1.is_none());
        let mut r = release();
        r["assets"][0]["browser_download_url"] = "https://evil.test/setup.exe".into();
        assert!(parse_release(&r, "0.1.25").is_err());
        let mut r = release();
        r["assets"].as_array_mut().unwrap().remove(1);
        assert!(parse_release(&r, "0.1.25").is_err());
        let mut r = release();
        r["prerelease"] = true.into();
        assert!(parse_release(&r, "0.1.25").is_err());
    }
    #[test]
    fn redirects_and_checksums_are_constrained() {
        for raw in [
            "http://github.com/MidiAug/xmu-sports/releases/download/v1/a",
            "https://github.com/other/repo/releases/download/v1/a",
            "https://github.com.evil.test/a",
            "https://evil@github.com/MidiAug/xmu-sports/releases/download/v1/a",
        ] {
            assert!(!allowed(&Url::parse(raw).unwrap()));
        }
        assert!(allowed(
            &Url::parse("https://release-assets.githubusercontent.com/test").unwrap()
        ));
        let line = format!("{}  installer.exe", "a".repeat(64));
        assert_eq!(checksum(&line, "installer.exe").unwrap(), "a".repeat(64));
        assert!(checksum(&format!("{line}\n{line}"), "installer.exe").is_err());
        assert!(checksum(&line, "other.exe").is_err());
    }
    #[test]
    fn mirrors_only_route_the_exact_official_installer() {
        let raw="https://github.com/MidiAug/xmu-sports/releases/download/v0.1.31/XMU.Sports_0.1.31_x64-setup.exe";
        assert_eq!(package_mirror(raw,None).unwrap(),None);
        assert_eq!(package_mirror(raw,Some("official")).unwrap(),None);
        for (route,prefix) in [("gh-proxy","https://gh-proxy.com/"),("ghproxy","https://ghproxy.net/")] {
            let routed=package_mirror(raw,Some(route)).unwrap().unwrap();
            assert_eq!(routed.as_str(),format!("{prefix}{raw}"));
            assert!(download_allowed(&routed,Some(&routed)));
            assert!(!allowed(&routed),"metadata must never inherit mirror access");
            assert!(!download_allowed(&Url::parse(&format!("{prefix}other")).unwrap(),Some(&routed)));
            assert!(!download_allowed(&Url::parse("https://evil.test/file").unwrap(),Some(&routed)));
            assert!(package_mirror(LATEST,Some(route)).is_err());
            assert!(package_mirror(&raw.replace("x64-setup.exe","SHA256SUMS.txt"),Some(route)).is_err());
            assert!(package_mirror(&raw.replace("MidiAug/xmu-sports","other/repo"),Some(route)).is_err());
            assert!(package_mirror(&format!("{raw}?token=secret"),Some(route)).is_err());
        }
        for route in ["http://gh-proxy.com/","https://evil.test/","other",""] {
            assert!(package_mirror(raw,Some(route)).is_err());
        }
    }
    #[test]
    fn partial_or_corrupted_download_is_never_an_installer() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("download.part");
        let hash = format!("{:x}", Sha256::digest(b"abc"));
        assert!(stream_package(&b"abc"[..], &file, 4, &hash, |_| {}).is_err());
        assert!(!file.exists());
        assert!(stream_package(&b"abc"[..], &file, 2, &hash, |_| {}).is_err());
        assert!(!file.exists());
        assert!(stream_package(&b"abd"[..], &file, 3, &hash, |_| {}).is_err());
        assert!(!file.exists());
        stream_package(&b"abc"[..], &file, 3, &hash, |_| {}).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"abc");
        assert!(stream_package(&b"xxx"[..], &file, 3, &hash, |_| {}).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"abc");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn automatic_pause_removes_partial_and_download_can_restart() {
        let dir=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());fs::create_dir_all(&dir).unwrap();
        let path=dir.join("download.part");let hash=format!("{:x}",Sha256::digest(b"abc"));
        let mut checks=0;
        let result=stream_package_checked(&b"abc"[..],&path,3,&hash,||{
            checks+=1;if checks>=3 {Err("UPDATE_PAUSED".into())}else{Ok(())}
        },|_|{});
        assert!(result.unwrap_err().contains("UPDATE_PAUSED"));assert!(!path.exists());
        stream_package(&b"abc"[..],&path,3,&hash,|_|{}).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn saved_download_requires_matching_official_candidate_and_valid_file() {
        let root=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let folder=root.join("download-id");fs::create_dir_all(&folder).unwrap();
        let path=folder.join("test.exe");fs::write(&path,b"abc").unwrap();
        let hash=format!("{:x}",Sha256::digest(b"abc"));
        let saved=root.join("downloaded.json");
        let mut item=Downloaded{path:path.clone(),size:3,sha256:hash.clone()};
        let candidate=Candidate{info:ReleaseInfo{status:"available".into(),version:"0.2.0".into(),notes:String::new(),size:3},name:"test.exe".into(),url:String::new(),sha256:hash};
        fs::write(&saved,serde_json::to_vec(&item).unwrap()).unwrap();
        assert!(cached_download(&saved,&root,&candidate).is_some());
        fs::write(&path,b"abd").unwrap();assert!(cached_download(&saved,&root,&candidate).is_none());
        fs::write(&path,b"abc").unwrap();item.path=root.join("outside.exe");
        fs::write(&saved,serde_json::to_vec(&item).unwrap()).unwrap();
        assert!(cached_download(&saved,&root,&candidate).is_none());
        fs::write(&saved,b"broken").unwrap();assert!(cached_download(&saved,&root,&candidate).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
