use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::sync::RwLock;

use crate::domain::Artist;
use crate::settings::data_path;

static LIBRARY: OnceLock<RwLock<Option<Arc<Vec<Artist>>>>> = OnceLock::new();

fn cache() -> &'static RwLock<Option<Arc<Vec<Artist>>>> {
    LIBRARY.get_or_init(|| RwLock::new(None))
}

pub async fn fetch_library() -> Result<Arc<Vec<Artist>>, Box<dyn Error + Send + Sync>> {
    if let Some(library) = cache().read().await.clone() {
        return Ok(library);
    }

    let mut library = tokio::task::spawn_blocking(|| {
        let pool = crate::persistence::connection::connect()?;
        let mut library = crate::library::indexer::export_library_from_database(&pool)?;
        apply_fetched_covers(&mut library, &fetched_covers(&pool));
        Ok::<_, Box<dyn Error + Send + Sync>>(library)
    })
    .await
    .map_err(|error| std::io::Error::other(format!("Library load task failed: {error}")))??;
    if library.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No indexed library is available.",
        )
        .into());
    }
    crate::library::normalize::normalize_library_data(&mut library);

    let library = Arc::new(library);
    *cache().write().await = Some(library.clone());
    Ok(library)
}

pub async fn store_library(library: Vec<Artist>) -> Arc<Vec<Artist>> {
    #[cfg(not(test))]
    let library = {
        let mut library = library;
        let covers = tokio::task::spawn_blocking(|| {
            crate::persistence::connection::connect()
                .map(|pool| fetched_covers(&pool))
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        apply_fetched_covers(&mut library, &covers);
        library
    };
    let library = Arc::new(library);
    *cache().write().await = Some(Arc::clone(&library));
    library
}

/// Covers fetched from the Cover Art Archive, by album id. Errors leave
/// albums as their files describe them.
fn fetched_covers(
    pool: &crate::persistence::connection::DbPool,
) -> std::collections::HashMap<String, String> {
    let rows = pool
        .get()
        .map_err(|error| error.to_string())
        .and_then(|mut connection| {
            crate::enrichment::store::load_cover_arts(&mut connection)
                .map_err(|error| error.to_string())
        });
    match rows {
        Ok(rows) => rows
            .into_iter()
            .map(|row| (row.album_id, row.file_path))
            .collect(),
        Err(error) => {
            tracing::debug!(%error, "no fetched album covers");
            std::collections::HashMap::new()
        }
    }
}

/// Albums with no artwork in their folder or tags show the front cover
/// fetched for their MusicBrainz release, when its file is still there.
pub fn apply_fetched_covers(
    library: &mut [Artist],
    covers: &std::collections::HashMap<String, String>,
) {
    if covers.is_empty() {
        return;
    }
    for album in library
        .iter_mut()
        .flat_map(|artist| artist.albums.iter_mut())
    {
        if !album.cover_url.trim().is_empty() {
            continue;
        }
        if let Some(path) = covers.get(&album.id)
            && std::path::Path::new(path).is_file()
        {
            album.cover_url = path.clone();
        }
    }
}

pub async fn refresh_cache() {
    *cache().write().await = None;
}

pub fn get_libraries_config_path() -> PathBuf {
    data_path(&["Config", "libraries.json"])
}

pub fn get_icon_art_path() -> PathBuf {
    data_path(&["Artist Icons"])
}

pub fn get_cover_art_path() -> PathBuf {
    data_path(&["Album Covers"])
}

pub fn get_profile_picture_path() -> PathBuf {
    data_path(&["Profile Pictures"])
}
