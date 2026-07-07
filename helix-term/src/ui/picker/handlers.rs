use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{atomic, Arc},
    time::Duration,
};

use helix_event::AsyncHook;
use helix_view::{document::from_reader, editor::FileExplorerConfig, Document};
use tokio::time::Instant;

use crate::{job, ui::overlay::Overlay};

use super::{CachedPreview, DynQueryCallback, Picker, MAX_FILE_SIZE_FOR_PREVIEW};

pub(super) struct PreviewLoadHandler<T: 'static + Send + Sync, D: 'static + Send + Sync> {
    phantom_data: std::marker::PhantomData<(T, D)>,
}

impl<T: 'static + Send + Sync, D: 'static + Send + Sync> Default for PreviewLoadHandler<T, D> {
    fn default() -> Self {
        Self {
            phantom_data: Default::default(),
        }
    }
}

impl<T: 'static + Send + Sync, D: 'static + Send + Sync> AsyncHook for PreviewLoadHandler<T, D> {
    type Event = Arc<Path>;

    fn handle_event(&mut self, path: Self::Event, _timeout: Option<Instant>) -> Option<Instant> {
        job::dispatch_blocking(move |editor, _compositor| {
            let file_explorer = editor.config().file_explorer.clone();
            let path_for_load = path.clone();

            tokio::spawn(async move {
                let path_for_update = path_for_load.clone();
                let loaded = tokio::task::spawn_blocking(move || {
                    load_preview(&path_for_load, &file_explorer)
                })
                .await
                .unwrap_or(LoadedPreview::NotFound);

                job::dispatch_blocking(move |editor, compositor| {
                    let Some(Overlay {
                        content: picker, ..
                    }) = compositor.find::<Overlay<Picker<T, D>>>()
                    else {
                        return;
                    };

                    let Some(cached_preview) = picker.preview_cache.get_mut(&path_for_update)
                    else {
                        return;
                    };

                    if !matches!(cached_preview, CachedPreview::Loading) {
                        return;
                    }

                    let (preview, highlight) = loaded.into_cached_preview(
                        &path_for_update,
                        editor.config.clone(),
                        editor.syn_loader.clone(),
                    );
                    *cached_preview = preview;
                    if highlight {
                        helix_event::send_blocking(
                            &picker.preview_highlight_handler,
                            path_for_update,
                        );
                    }
                    helix_event::request_redraw();
                });
            });
        });

        None
    }

    fn finish_debounce(&mut self) {}
}

enum LoadedPreview {
    Document {
        text: helix_core::Rope,
        encoding: &'static helix_core::encoding::Encoding,
        has_bom: bool,
    },
    Directory(Vec<(String, bool)>),
    Binary,
    LargeFile,
    NotFound,
}

impl LoadedPreview {
    fn into_cached_preview(
        self,
        path: &Path,
        config: std::sync::Arc<dyn arc_swap::access::DynAccess<helix_view::editor::Config>>,
        syn_loader: std::sync::Arc<arc_swap::ArcSwap<helix_core::syntax::Loader>>,
    ) -> (CachedPreview, bool) {
        match self {
            LoadedPreview::Document {
                text,
                encoding,
                has_bom,
            } => {
                let mut doc =
                    Document::from(text, Some((encoding, has_bom)), config, syn_loader.clone());
                doc.set_path(Some(path));
                let loader = syn_loader.load();
                let highlight = if let Some(language_config) = doc.detect_language_config(&loader) {
                    doc.language = Some(language_config);
                    true
                } else {
                    false
                };
                doc.detect_indent_and_line_ending();
                (CachedPreview::Document(Box::new(doc)), highlight)
            }
            LoadedPreview::Directory(entries) => (CachedPreview::Directory(entries), false),
            LoadedPreview::Binary => (CachedPreview::Binary, false),
            LoadedPreview::LargeFile => (CachedPreview::LargeFile, false),
            LoadedPreview::NotFound => (CachedPreview::NotFound, false),
        }
    }
}

pub(super) struct PreviewHighlightHandler<T: 'static + Send + Sync, D: 'static + Send + Sync> {
    trigger: Option<Arc<Path>>,
    phantom_data: std::marker::PhantomData<(T, D)>,
}

impl<T: 'static + Send + Sync, D: 'static + Send + Sync> Default for PreviewHighlightHandler<T, D> {
    fn default() -> Self {
        Self {
            trigger: None,
            phantom_data: Default::default(),
        }
    }
}

fn load_preview(path: &Path, file_explorer: &FileExplorerConfig) -> LoadedPreview {
    std::fs::metadata(path)
        .and_then(|metadata| {
            if metadata.is_dir() {
                return directory_preview_content(path, file_explorer)
                    .map(LoadedPreview::Directory);
            }

            if !metadata.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Neither a dir, nor a file",
                ));
            }

            if metadata.len() > MAX_FILE_SIZE_FOR_PREVIEW {
                return Ok(LoadedPreview::LargeFile);
            }
            let mut read_buffer = Vec::with_capacity(1024);
            let is_binary = std::fs::File::open(path).and_then(|file| {
                // Read up to 1kb to detect the content type.
                let n = file.take(1024).read_to_end(&mut read_buffer)?;
                Ok(crate::is_binary(&read_buffer[..n]))
            })?;
            if is_binary {
                return Ok(LoadedPreview::Binary);
            }

            let mut file = std::fs::File::open(path)?;
            let (text, encoding, has_bom) = from_reader(&mut file, None)?;
            Ok(LoadedPreview::Document {
                text,
                encoding,
                has_bom,
            })
        })
        .unwrap_or(LoadedPreview::NotFound)
}

fn directory_preview_content(
    root: &Path,
    config: &FileExplorerConfig,
) -> Result<Vec<(String, bool)>, std::io::Error> {
    let files = directory_content(root, config)?;
    Ok(files
        .iter()
        .filter_map(|(file_path, is_dir)| {
            let name = file_path
                .strip_prefix(root)
                .map(|p| Some(p.as_os_str()))
                .unwrap_or_else(|_| file_path.file_name())?
                .to_string_lossy();
            if *is_dir {
                Some((format!("{}/", name), true))
            } else {
                Some((name.into_owned(), false))
            }
        })
        .collect())
}

fn directory_content(
    root: &Path,
    config: &FileExplorerConfig,
) -> Result<Vec<(PathBuf, bool)>, std::io::Error> {
    use ignore::WalkBuilder;

    let mut walk_builder = WalkBuilder::new(root);

    let mut content: Vec<(PathBuf, bool)> = walk_builder
        .hidden(config.hidden)
        .parents(config.parents)
        .ignore(config.ignore)
        .follow_links(config.follow_symlinks)
        .git_ignore(config.git_ignore)
        .git_global(config.git_global)
        .git_exclude(config.git_exclude)
        .max_depth(Some(1))
        .add_custom_ignore_filename(helix_loader::config_dir().join("ignore"))
        .add_custom_ignore_filename(".helix/ignore")
        .types(excluded_types())
        .build()
        .filter_map(|entry| {
            entry
                .map(|entry| {
                    let path = entry.path();
                    let is_dir = path.is_dir();
                    let mut path = path.to_path_buf();
                    if is_dir && path != root && config.flatten_dirs {
                        while let Some(single_child_directory) = get_child_if_single_dir(&path) {
                            path = single_child_directory;
                        }
                    }
                    (path, is_dir)
                })
                .ok()
                .filter(|entry| entry.0 != root)
        })
        .collect();

    content.sort_by(|(path1, is_dir1), (path2, is_dir2)| (!is_dir1, path1).cmp(&(!is_dir2, path2)));

    if root.parent().is_some() {
        content.insert(0, (root.join(".."), true));
    }

    Ok(content)
}

fn get_child_if_single_dir(path: &Path) -> Option<PathBuf> {
    let mut entries = path.read_dir().ok()?;
    let entry = entries.next()?.ok()?;
    let entry_path = entry.path();
    if entries.next().is_none() && entry_path.is_dir() {
        Some(entry_path)
    } else {
        None
    }
}

fn excluded_types() -> ignore::types::Types {
    use ignore::types::TypesBuilder;
    let mut type_builder = TypesBuilder::new();
    type_builder
        .add(
            "compressed",
            "*.{zip,gz,bz2,zst,lzo,sz,tgz,tbz2,lz,lz4,lzma,lzo,z,Z,xz,7z,rar,cab}",
        )
        .expect("Invalid type definition");
    type_builder.negate("all");
    type_builder
        .build()
        .expect("failed to build excluded_types")
}

impl<T: 'static + Send + Sync, D: 'static + Send + Sync> AsyncHook
    for PreviewHighlightHandler<T, D>
{
    type Event = Arc<Path>;

    fn handle_event(
        &mut self,
        path: Self::Event,
        timeout: Option<tokio::time::Instant>,
    ) -> Option<tokio::time::Instant> {
        if self
            .trigger
            .as_ref()
            .is_some_and(|trigger| trigger == &path)
        {
            // If the path hasn't changed, don't reset the debounce
            timeout
        } else {
            self.trigger = Some(path);
            Some(Instant::now() + Duration::from_millis(150))
        }
    }

    fn finish_debounce(&mut self) {
        let Some(path) = self.trigger.take() else {
            return;
        };

        job::dispatch_blocking(move |editor, compositor| {
            let Some(Overlay {
                content: picker, ..
            }) = compositor.find::<Overlay<Picker<T, D>>>()
            else {
                return;
            };

            let Some(CachedPreview::Document(ref mut doc)) = picker.preview_cache.get_mut(&path)
            else {
                return;
            };

            if doc.syntax().is_some() {
                return;
            }

            let Some(language) = doc.language_config().map(|config| config.language()) else {
                return;
            };

            let syn_loader = editor.syn_loader.clone();
            let text = doc.text().clone();

            tokio::task::spawn_blocking(move || {
                let loader = syn_loader.load();
                let syntax = match helix_core::Syntax::new(text.slice(..), language, &loader) {
                    Ok(syntax) => syntax,
                    Err(err) => {
                        log::info!("highlighting picker preview failed: {err}");
                        return;
                    }
                };

                job::dispatch_blocking(move |editor, compositor| {
                    let Some(Overlay {
                        content: picker, ..
                    }) = compositor.find::<Overlay<Picker<T, D>>>()
                    else {
                        log::info!("picker closed before syntax highlighting finished");
                        return;
                    };
                    let Some(CachedPreview::Document(ref mut doc)) =
                        picker.preview_cache.get_mut(&path)
                    else {
                        return;
                    };
                    let diagnostics = helix_view::Editor::doc_diagnostics(
                        &editor.language_servers,
                        &editor.diagnostics,
                        doc,
                    );
                    doc.replace_diagnostics(diagnostics, &[], None);
                    doc.syntax = Some(syntax);
                });
            });
        });
    }
}

pub(super) struct DynamicQueryChange {
    pub query: Arc<str>,
    pub is_paste: bool,
}

pub(super) struct DynamicQueryHandler<T: 'static + Send + Sync, D: 'static + Send + Sync> {
    callback: Arc<DynQueryCallback<T, D>>,
    // Duration used as a debounce.
    // Defaults to 100ms if not provided via `Picker::with_dynamic_query`. Callers may want to set
    // this higher if the dynamic query is expensive - for example global search.
    debounce: Duration,
    last_query: Arc<str>,
    query: Option<Arc<str>>,
}

impl<T: 'static + Send + Sync, D: 'static + Send + Sync> DynamicQueryHandler<T, D> {
    pub(super) fn new(callback: DynQueryCallback<T, D>, duration_ms: Option<u64>) -> Self {
        Self {
            callback: Arc::new(callback),
            debounce: Duration::from_millis(duration_ms.unwrap_or(100)),
            last_query: "".into(),
            query: None,
        }
    }
}

impl<T: 'static + Send + Sync, D: 'static + Send + Sync> AsyncHook for DynamicQueryHandler<T, D> {
    type Event = DynamicQueryChange;

    fn handle_event(&mut self, change: Self::Event, _timeout: Option<Instant>) -> Option<Instant> {
        let DynamicQueryChange { query, is_paste } = change;
        if query == self.last_query {
            // If the search query reverts to the last one we requested, no need to
            // make a new request.
            self.query = None;
            None
        } else {
            self.query = Some(query);
            if is_paste {
                self.finish_debounce();
                None
            } else {
                Some(Instant::now() + self.debounce)
            }
        }
    }

    fn finish_debounce(&mut self) {
        let Some(query) = self.query.take() else {
            return;
        };
        self.last_query = query.clone();
        let callback = self.callback.clone();

        job::dispatch_blocking(move |editor, compositor| {
            let Some(Overlay {
                content: picker, ..
            }) = compositor.find::<Overlay<Picker<T, D>>>()
            else {
                return;
            };
            // Increment the version number to cancel any ongoing requests.
            picker.version.fetch_add(1, atomic::Ordering::Relaxed);
            picker.matcher.restart(false);
            let injector = picker.injector();
            let get_options = (callback)(&query, editor, picker.editor_data.clone(), &injector);
            tokio::spawn(async move {
                if let Err(err) = get_options.await {
                    log::info!("Dynamic request failed: {err}");
                }
                // NOTE: the Drop implementation of Injector will request a redraw when the
                // injector falls out of scope here, clearing the "running" indicator.
            });
        })
    }
}
