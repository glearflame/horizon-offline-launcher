//! Atalho para jogar uma versão customizada do Minecraft (ex.: 26.3) fora do fluxo oficial do
//! launcher.
//!
//! Baixa um `.zip` de uma URL remota, extrai a árvore completa (`versions/`, `libraries/`,
//! `assets/`, ...) e deixa o launcher apontar uma instância Java para essa pasta.
//!
//! Hospede o `26.3.zip` em qualquer host remoto (GitHub Releases, seu próprio servidor, etc.) e
//! aponte a variável `HORIZON_CUSTOM_JAR_URL` para ele. O download vai para
//! `~/Downloads/26.3.zip` (sobrescritível com `HORIZON_CUSTOM_JAR_DEST`) e a extração fica na
//! pasta de dados do launcher, em `custom-jar/sdas/`.

use std::io::copy;
use std::path::{Path, PathBuf};

/// Resultado da última operação dos botões customizados.
#[derive(Clone, Debug, Default)]
pub struct CustomJarState {
    /// A última mensagem: `"baixando…"`, `"extraído em <caminho>"` ou `"erro: <msg>"`.
    pub status: Option<String>,
    /// Caminho da pasta `sdas/` extraída (a raiz com `versions/`, `libraries/`, etc.).
    pub extracted_dir: Option<PathBuf>,
}

impl CustomJarState {
    pub fn ok(msg: impl Into<String>, dir: PathBuf) -> Self {
        Self {
            status: Some(msg.into()),
            extracted_dir: Some(dir),
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            status: Some(msg.into()),
            extracted_dir: None,
        }
    }
}

/// Pasta base da extração customizada (dentro da pasta de dados do launcher).
pub fn custom_home() -> PathBuf {
    crate::backend::data_dir().join("custom-jar")
}

/// Pasta `sdas/` extraída (raiz com `versions/26.3/26.3.jar` etc.).
pub fn extracted_dir() -> PathBuf {
    custom_home().join("sdas")
}

/// URL do zip. Padrão: o release `v26.3` no GitHub (`glearflame/horizon-263`).
/// Sobreponha com a variável de ambiente `HORIZON_CUSTOM_JAR_URL`.
pub fn zip_url() -> Option<String> {
    std::env::var("HORIZON_CUSTOM_JAR_URL")
        .ok()
        .or_else(|| Some("https://github.com/glearflame/horizon-263/releases/download/v26.3/26.3.zip".to_string()))
}

/// Destino do download do zip. Padrão: `~/Downloads/26.3.zip`.
pub fn zip_dest() -> PathBuf {
    match std::env::var("HORIZON_CUSTOM_JAR_DEST").ok() {
        Some(v) => PathBuf::from(v),
        None => dirs::download_dir()
            .unwrap_or_else(|| std::env::temp_dir())
            .join("26.3.zip"),
    }
}

/// Nome da pasta raiz dentro do zip (o 26.3 do pixeldrain tem `sdas/`).
const ZIP_ROOT: &str = "sdas";

/// Baixa o zip de `url` para `dest`.
pub fn download_zip(url: &str, dest: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("criar pasta: {e}"))?;
    }
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("baixar {url}: {e}"))?;
    let mut out = std::fs::File::create(dest).map_err(|e| format!("criar {dest:?}: {e}"))?;
    copy(&mut resp.into_reader(), &mut out).map_err(|e| format!("gravar: {e}"))?;
    Ok(dest.to_path_buf())
}

/// Extrai TUDO do zip para `out_dir`, preservando a estrutura interna (incluindo a pasta `sdas/`).
pub fn extract_all(zip: &Path, out_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("criar pasta: {e}"))?;
    let file = std::fs::File::open(zip).map_err(|e| format!("abrir zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("ler zip: {e}"))?;
    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            continue;
        };
        let name = match entry.name() {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };
        let out_path = out_dir.join(PathBuf::from(name.clone()));
        if name.ends_with('/') {
            std::fs::create_dir_all(&out_path).map_err(|e| format!("criar pasta {name}: {e}"))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("criar pasta: {e}"))?;
            }
            let mut out = std::fs::File::create(&out_path).map_err(|e| format!("criar {name}: {e}"))?;
            copy(&mut entry, &mut out).map_err(|e| format!("gravar {name}: {e}"))?;
        }
    }
    Ok(out_dir.join(ZIP_ROOT))
}

/// Baixa o zip configurado e extrai tudo para `custom-jar/sdas/`. Retorna o estado.
pub fn ensure_extracted() -> CustomJarState {
    let Some(url) = zip_url() else {
        return CustomJarState::err(
            "defina HORIZON_CUSTOM_JAR_URL com a URL do 26.3.zip (um host remoto, não local)",
        );
    };
    let dest = zip_dest();
    let target = extracted_dir();
    // Já extraído antes: não baixa de novo.
    if target.join("versions").join("26.3").join("26.3.jar").is_file() {
        return CustomJarState::ok("26.3 já extraído em", target);
    }
    match download_zip(&url, &dest) {
        Ok(_) => match extract_all(&dest, &custom_home()) {
            Ok(dir) => CustomJarState::ok("26.3 extraído em", dir),
            Err(e) => CustomJarState::err(format!("extração falhou: {e}")),
        },
        Err(e) => CustomJarState::err(format!("download falhou: {e}")),
    }
}

/// Apaga a extração e o zip de download.
#[allow(dead_code)]
pub fn clear() -> Result<(), String> {
    let _ = std::fs::remove_dir_all(extracted_dir());
    let _ = std::fs::remove_file(zip_dest());
    Ok(())
}
