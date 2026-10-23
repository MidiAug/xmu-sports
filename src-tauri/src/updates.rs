//! Public release downloads are isolated from the authenticated booking worker.
use reqwest::{
    blocking::{Client, Response},
    Url,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};

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
#[derive(Default)]
struct UpdateState {
    cached: Option<(Instant, Result<ReleaseInfo, String>)>,
    candidate: Option<Candidate>,
    downloaded: Option<PathBuf>,
}
static STATE: OnceLock<Mutex<UpdateState>> = OnceLock::new();
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
    let mut url = Url::parse(raw).map_err(|_| "更新地址无效")?;
    for _ in 0..6 {
        if !allowed(&url) {
            return Err("更新地址不受信任，请查看发布页".into());
        }
        let response = client
            .get(url.clone())
            .timeout(Duration::from_secs(seconds))
            .send()
            .map_err(|_| "无法连接 GitHub，请检查网络后重试".to_string())?;
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
    let mut state = state()
        .try_lock()
        .map_err(|_| "正在检查或下载更新，请稍候")?;
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
    state.cached = Some((Instant::now(), result.clone()));
    result
}

fn stream_package(
    mut source: impl Read,
    dest: &Path,
    size: u64,
    hash: &str,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
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
            let len = source.read(&mut buffer).map_err(|_| "下载中断，请重试")?;
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
#[tauri::command]
pub async fn download_app_update(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut state = state()
            .try_lock()
            .map_err(|_| "正在检查或下载更新，请稍候")?;
        let candidate = state.candidate.as_ref().ok_or("请先检查更新")?;
        let folder = app
            .path()
            .download_dir()
            .map_err(|_| "无法访问下载文件夹")?
            .join("XMU Sports Updates")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&folder).map_err(|_| "无法创建下载文件夹")?;
        let part = folder.join("download.part");
        let final_path = folder.join(&candidate.name);
        let response = open(&client()?, &candidate.url, 600)?;
        let mut last = Instant::now() - Duration::from_secs(1);
        stream_package(
            response,
            &part,
            candidate.info.size,
            &candidate.sha256,
            |received| {
                if last.elapsed() >= Duration::from_millis(150) || received == candidate.info.size {
                    let _ = app.emit(
                        "app-update-progress",
                        serde_json::json!({"received":received,"total":candidate.info.size}),
                    );
                    last = Instant::now();
                }
            },
        )?;
        if let Err(_) = fs::rename(&part, &final_path) {
            let _ = fs::remove_file(&part);
            return Err("无法保存校验后的安装包".into());
        }
        state.downloaded = Some(final_path.clone());
        Ok(final_path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "更新下载中断")?
}
#[tauri::command]
pub fn open_update_folder(app: tauri::AppHandle) -> Result<(), String> {
    let state = state()
        .try_lock()
        .map_err(|_| "正在检查或下载更新，请稍候")?;
    let folder = state
        .downloaded
        .as_ref()
        .and_then(|p| p.parent())
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
