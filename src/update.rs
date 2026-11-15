//! Automatic updates from the project's GitHub releases.
//!
//! Hearth asks GitHub for the newest release of the repository below. If its version (the
//! release tag, like `v0.3.0`) is newer than this program, it downloads the release's
//! program (an `.exe`, or a `.zip` holding one), checks it, and keeps it ready. The new
//! program replaces this one when the person presses "Restart to update", or when they close
//! Hearth if automatic install is on. If the release also has a `<file>.sha256` or
//! `SHA256SUMS` file, the download must match it.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const REPO: &str = "NamiSwan222/fixcord";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Largest download accepted.
const MAX_DOWNLOAD: u64 = 300 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    NoReleases,
    Available { version: String, notes: String },
    Downloading { version: String, done: u64, total: u64 },
    Ready { version: String, notes: String, file: PathBuf },
    Failed(String),
}

#[derive(Clone)]
pub struct Updater {
    state: Arc<Mutex<State>>,
}

#[derive(Clone)]
struct Asset {
    name: String,
    url: String,
    size: u64,
}

/// "v1.2.3" / "1.2" → [1, 2, 3]
pub fn parse_version(s: &str) -> Vec<u64> {
    s.trim()
        .trim_start_matches(['v', 'V'])
        .split(|c: char| c == '.' || c == '-' || c == '+')
        .map_while(|p| p.parse::<u64>().ok())
        .collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse_version(candidate), parse_version(current));
    if a.is_empty() {
        return false;
    }
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .build()
        .into()
}

fn get_text(url: &str) -> Result<(u16, String), String> {
    let resp = agent()
        .get(url)
        .header("User-Agent", "Hearth-updater")
        .header("Accept", "application/vnd.github+json")
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().map_err(|e| format!("Couldn't read GitHub's answer: {e}"))?;
    Ok((status, text))
}

impl Updater {
    pub fn new() -> Updater {
        Updater { state: Arc::new(Mutex::new(State::Idle)) }
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    fn set(&self, s: State) {
        *self.state.lock().unwrap() = s;
    }

    pub fn busy(&self) -> bool {
        matches!(self.state(), State::Checking | State::Downloading { .. })
    }

    /// Ask GitHub for the newest release (in the background). With `download`, fetch it too.
    pub fn check(&self, download: bool) {
        if self.busy() || matches!(self.state(), State::Ready { .. }) {
            return;
        }
        self.set(State::Checking);
        let me = self.clone();
        std::thread::spawn(move || {
            match me.find_release() {
                Ok(None) => {}
                Ok(Some((version, notes, assets))) => {
                    me.set(State::Available { version: version.clone(), notes: notes.clone() });
                    if download {
                        me.fetch(version, notes, assets);
                    }
                }
                Err(e) => me.set(State::Failed(e)),
            }
        });
    }

    /// Download the release that `check` found.
    pub fn download(&self) {
        if self.busy() {
            return;
        }
        let me = self.clone();
        self.set(State::Checking);
        std::thread::spawn(move || match me.find_release() {
            Ok(Some((version, notes, assets))) => me.fetch(version, notes, assets),
            Ok(None) => {}
            Err(e) => me.set(State::Failed(e)),
        });
    }

    fn find_release(&self) -> Result<Option<(String, String, Vec<Asset>)>, String> {
        let (status, text) = get_text(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
        if status == 404 {
            self.set(State::NoReleases);
            return Ok(None);
        }
        if status != 200 {
            return Err(format!("GitHub answered with code {status}. Try again later."));
        }
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|_| "GitHub sent something unexpected.".to_string())?;
        let tag = json["tag_name"].as_str().unwrap_or_default().to_string();
        let notes: String = json["body"].as_str().unwrap_or_default().chars().take(2000).collect();
        if !is_newer(&tag, CURRENT) {
            self.set(State::UpToDate);
            return Ok(None);
        }
        let assets: Vec<Asset> = json["assets"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|a| {
                        Some(Asset {
                            name: a["name"].as_str()?.to_string(),
                            url: a["browser_download_url"].as_str()?.to_string(),
                            size: a["size"].as_u64().unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some((tag.trim_start_matches(['v', 'V']).to_string(), notes, assets)))
    }

    fn fetch(&self, version: String, notes: String, assets: Vec<Asset>) {
        match self.fetch_inner(&version, &assets) {
            Ok(file) => self.set(State::Ready { version, notes, file }),
            Err(e) => self.set(State::Failed(e)),
        }
    }

    fn fetch_inner(&self, version: &str, assets: &[Asset]) -> Result<PathBuf, String> {
        let lower = |a: &Asset| a.name.to_lowercase();
        let pick = assets
            .iter()
            .find(|a| lower(a).ends_with(".exe") && lower(a).contains("hearth"))
            .or_else(|| assets.iter().find(|a| lower(a).ends_with(".exe")))
            .or_else(|| assets.iter().find(|a| lower(a).ends_with(".zip") && lower(a).contains("hearth")))
            .or_else(|| assets.iter().find(|a| lower(a).ends_with(".zip")))
            .ok_or_else(|| format!("Version {version} has no Hearth program attached (an .exe or .zip)."))?
            .clone();
        if pick.size > MAX_DOWNLOAD {
            return Err("The update is unusually large, so it wasn't downloaded.".into());
        }
        let bytes = self.download_bytes(version, &pick)?;

        // Checksum, when the release publishes one.
        let sum_asset = assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(&format!("{}.sha256", pick.name)))
            .or_else(|| assets.iter().find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS") || a.name.eq_ignore_ascii_case("SHA256SUMS.txt")));
        if let Some(sum) = sum_asset {
            let (status, text) = get_text(&sum.url)?;
            if status != 200 {
                return Err("Couldn't download the update's checksum.".into());
            }
            let expected = text
                .lines()
                .find(|l| l.contains(&pick.name) || text.lines().count() == 1)
                .and_then(|l| l.split_whitespace().next())
                .map(|s| s.to_lowercase())
                .ok_or("The checksum file doesn't mention the download.")?;
            let actual = hex(&Sha256::digest(&bytes));
            if actual != expected {
                return Err("The download didn't match its checksum, so it wasn't used.".into());
            }
        }

        let exe = if pick.name.to_lowercase().ends_with(".zip") { exe_from_zip(&bytes)? } else { bytes };
        if exe.len() < 512 * 1024 || &exe[..2] != b"MZ" {
            return Err("The download isn't a Windows program, so it wasn't used.".into());
        }
        let dir = std::env::temp_dir().join("hearth-update");
        std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't save the update: {e}"))?;
        let file = dir.join(format!("Hearth-{version}.exe"));
        std::fs::write(&file, &exe).map_err(|e| format!("Couldn't save the update: {e}"))?;
        Ok(file)
    }

    fn download_bytes(&self, version: &str, a: &Asset) -> Result<Vec<u8>, String> {
        let resp = agent()
            .get(&a.url)
            .header("User-Agent", "Hearth-updater")
            .call()
            .map_err(|e| format!("Download failed: {e}"))?;
        let mut reader = resp.into_body().into_reader();
        let mut out = Vec::with_capacity(a.size as usize);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("Download failed: {e}"))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            if out.len() as u64 > MAX_DOWNLOAD {
                return Err("The update is unusually large, so it wasn't downloaded.".into());
            }
            self.set(State::Downloading { version: version.to_string(), done: out.len() as u64, total: a.size });
        }
        Ok(out)
    }

    /// Put the downloaded program in place of this one. Returns the program's path.
    pub fn install(&self) -> Result<PathBuf, String> {
        let State::Ready { file, .. } = self.state() else {
            return Err("No update is ready.".into());
        };
        let current = std::env::current_exe().map_err(|e| e.to_string())?;
        replace_program(&current, &file)?;
        let _ = std::fs::remove_file(&file);
        self.set(State::UpToDate);
        Ok(current)
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn exe_from_zip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "The update zip couldn't be opened.".to_string())?;
    let mut best: Option<usize> = None;
    for i in 0..z.len() {
        let Ok(f) = z.by_index(i) else { continue };
        let name = f.name().map(|n| n.to_lowercase()).unwrap_or_default();
        if name.ends_with(".exe") && (best.is_none() || name.contains("hearth")) {
            best = Some(i);
        }
    }
    let i = best.ok_or("The update zip has no program inside.")?;
    let mut f = z.by_index(i).map_err(|e| e.to_string())?;
    if f.size() > MAX_DOWNLOAD {
        return Err("The program in the update zip is unusually large.".into());
    }
    let mut out = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Windows lets a running program be renamed but not overwritten: move this one aside as
/// `<name>.old.exe`, then copy the new one into its place.
fn replace_program(current: &Path, new_file: &Path) -> Result<(), String> {
    let old = old_path(current);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(current, &old).map_err(|e| format!("Couldn't move the old version aside: {e}"))?;
    if let Err(e) = std::fs::copy(new_file, current) {
        let _ = std::fs::rename(&old, current); // put it back
        return Err(format!("Couldn't put the new version in place: {e}"));
    }
    Ok(())
}

fn old_path(current: &Path) -> PathBuf {
    let stem = current.file_stem().and_then(|s| s.to_str()).unwrap_or("Hearth");
    current.with_file_name(format!("{stem}.old.exe"))
}

/// Remove the previous version left behind by an update (it can't delete itself while running).
pub fn cleanup_after_update() {
    if let Ok(current) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&current));
    }
}

/// Start this program again (after an update) — the caller then closes this window.
pub fn restart(program: &Path) -> Result<(), String> {
    std::process::Command::new(program).spawn().map(|_| ()).map_err(|e| format!("Couldn't restart Hearth: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_like_people_expect() {
        assert!(is_newer("v0.3.0", "0.2.0"));
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("latest", "0.2.0"));
        assert_eq!(parse_version("v1.2.3-beta"), vec![1, 2, 3]);
    }

    #[test]
    fn program_is_found_inside_a_zip() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            w.start_file("Hearth/READ ME.txt", opts).unwrap();
            std::io::Write::write_all(&mut w, b"hello").unwrap();
            w.start_file("Hearth/Hearth.exe", opts).unwrap();
            std::io::Write::write_all(&mut w, b"MZ-program").unwrap();
            w.finish().unwrap();
        }
        assert_eq!(exe_from_zip(buf.get_ref()).unwrap(), b"MZ-program");
    }

    #[test]
    fn replacing_a_program_keeps_the_old_one_aside() {
        let dir = std::env::temp_dir().join(format!("hearth-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let current = dir.join("Hearth.exe");
        let new = dir.join("new.exe");
        std::fs::write(&current, b"old version").unwrap();
        std::fs::write(&new, b"new version").unwrap();
        replace_program(&current, &new).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"new version");
        assert_eq!(std::fs::read(old_path(&current)).unwrap(), b"old version");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
