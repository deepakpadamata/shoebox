//! S2-02: where a device gets a search model file when its own library cannot give it: the
//! device's own folders, then a device that offers it, then a download that is kept only when it is
//! the model.

mod common;

use common::*;
use shoebox_node::{download_model_from, ModelSources};
use shoebox_search::models::ModelFile;
use shoebox_testkit::{wait_until, Cluster, FileServer};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shoebox_core::model::Role;

fn bytes(seed: u32, len: u32) -> Vec<u8> {
    (0..len)
        .map(|i| (i.wrapping_mul(7).wrapping_add(seed) % 251) as u8)
        .collect()
}

/// A model file for `body`, named `name`, published at `url`.
fn model(name: &'static str, body: &[u8], url: &str) -> ModelFile {
    ModelFile {
        name,
        blake3: *blake3::hash(body).as_bytes(),
        size: body.len() as u64,
        url: Box::leak(url.to_string().into_boxed_str()),
    }
}

fn no_progress() -> impl Fn(u64, u64) + Send + Sync {
    |_, _| {}
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_download_is_kept_when_it_is_the_model_and_progress_is_reported() {
    let body = bytes(1, 300_000);
    let server = FileServer::serve(body.clone()).start().await;
    let file = model("m.onnx", &body, &server.url());
    let dir = tempfile::tempdir().unwrap();
    let seen = Mutex::new(vec![]);
    let path = download_model_from(&server.url(), &file, dir.path(), &|d, t| {
        seen.lock().unwrap().push((d, t));
    })
    .await
    .unwrap();
    assert_eq!(path, dir.path().join("m.onnx"));
    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert_eq!(files_in(dir.path()), ["m.onnx"], "no .part left behind");
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.last(), Some(&(300_000, 300_000)));
    assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_download_that_is_not_the_model_is_refused_and_nothing_is_kept() {
    let right = bytes(1, 100_000);
    // Same size, other bytes: only the hash can tell.
    let wrong = bytes(2, 100_000);
    let server = FileServer::serve(wrong).start().await;
    let file = model("m.onnx", &right, &server.url());
    let dir = tempfile::tempdir().unwrap();
    let err = download_model_from(&server.url(), &file, dir.path(), &no_progress())
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("not the model"), "{err:#}");
    assert!(
        files_in(dir.path()).is_empty(),
        "{:?}",
        files_in(dir.path())
    );

    // Longer than the model: stopped at the size, not stored.
    let long = FileServer::serve(bytes(1, 150_000)).start().await;
    let err = download_model_from(&long.url(), &file, dir.path(), &no_progress())
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("larger"), "{err:#}");
    assert!(files_in(dir.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_interrupted_download_resumes_where_it_stopped() {
    let body = bytes(3, 400_000);
    let server = FileServer::serve(body.clone())
        .cut_first_response_at(150_000)
        .start()
        .await;
    let file = model("m.onnx", &body, &server.url());
    let dir = tempfile::tempdir().unwrap();
    download_model_from(&server.url(), &file, dir.path(), &no_progress())
        .await
        .expect_err("the connection is dropped part-way");
    let part = dir.path().join("m.onnx.part");
    let have = std::fs::metadata(&part).unwrap().len();
    assert!(have > 0 && have < 400_000, "{have} bytes kept");
    assert!(!dir.path().join("m.onnx").exists());

    let path = download_model_from(&server.url(), &file, dir.path(), &no_progress())
        .await
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
    assert_eq!(
        server.requests(),
        [None, Some(have)],
        "the second request asked only for the rest"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_cannot_resume_is_started_over() {
    let body = bytes(4, 200_000);
    let server = FileServer::serve(body.clone())
        .cut_first_response_at(50_000)
        .ignoring_ranges()
        .start()
        .await;
    let file = model("m.onnx", &body, &server.url());
    let dir = tempfile::tempdir().unwrap();
    download_model_from(&server.url(), &file, dir.path(), &no_progress())
        .await
        .expect_err("cut");
    let path = download_model_from(&server.url(), &file, dir.path(), &no_progress())
        .await
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_model_comes_from_the_devices_own_folder_then_a_peer_then_a_download() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let (keeper, phone) = (&nodes[0], &nodes[1]);
    let work = tempfile::tempdir().unwrap();
    let progress: Arc<Mutex<u32>> = Arc::default();
    let sources = |dirs: Vec<std::path::PathBuf>, download: bool, name: &str| ModelSources {
        dirs,
        download_to: download.then(|| work.path().join(format!("dl-{name}"))),
        progress: Some({
            let progress = progress.clone();
            Arc::new(move |_, _, _| *progress.lock().unwrap() += 1)
        }),
    };

    // 1. The device's own folder wins over everything, and a file there that is not the model is
    //    ignored. The server is never asked.
    let body_a = bytes(10, 120_000);
    let server_a = FileServer::serve(body_a.clone()).start().await;
    let a = model("a.onnx", &body_a, &server_a.url());
    let own = work.path().join("own");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("a.onnx"), bytes(11, 120_000)).unwrap();
    let got = phone
        .model_bytes_from(&a, &sources(vec![own.clone()], true, "a"))
        .await
        .unwrap()
        .expect("downloaded, since the file in the folder is not the model");
    assert_eq!(&got[..], &body_a[..]);
    assert_eq!(
        server_a.requests().len(),
        1,
        "the wrong local file was skipped"
    );
    std::fs::write(own.join("a.onnx"), &body_a).unwrap();
    let b_body = bytes(12, 90_000);
    let server_b = FileServer::serve(b_body.clone()).start().await;
    let b = model("b.onnx", &b_body, &server_b.url());
    std::fs::write(own.join("b.onnx"), &b_body).unwrap();
    let got = phone
        .model_bytes_from(&b, &sources(vec![own.clone()], true, "b"))
        .await
        .unwrap()
        .expect("from the folder");
    assert_eq!(&got[..], &b_body[..]);
    assert!(
        server_b.requests().is_empty(),
        "a local copy needs no download"
    );

    // 2. A device of the library that offers it beats a download.
    let body_c = bytes(13, 200_000);
    let server_c = FileServer::serve(body_c.clone()).start().await;
    let cfile = model("c.onnx", &body_c, &server_c.url());
    let offered = work.path().join("offered");
    std::fs::create_dir_all(&offered).unwrap();
    std::fs::write(offered.join("c.onnx"), &body_c).unwrap();
    keeper
        .share_model(&cfile, &offered.join("c.onnx"))
        .await
        .unwrap();
    wait_until(Duration::from_secs(30), || async {
        phone.model_offered(&cfile).await.unwrap()
    })
    .await
    .expect("the offer reaches the phone");
    let got = phone
        .model_bytes_from(&cfile, &sources(vec![], true, "c"))
        .await
        .unwrap()
        .expect("from the keeper");
    assert_eq!(&got[..], &body_c[..]);
    assert!(
        server_c.requests().is_empty(),
        "a peer's copy needs no download"
    );

    // 3. Nobody has it: download, only where that is allowed, then offer it on.
    let body_d = bytes(14, 150_000);
    let server_d = FileServer::serve(body_d.clone()).start().await;
    let d = model("d.onnx", &body_d, &server_d.url());
    assert_eq!(
        phone
            .model_bytes_from(&d, &sources(vec![], false, "d"))
            .await
            .unwrap(),
        None,
        "no download on a metered network"
    );
    assert!(server_d.requests().is_empty());
    let got = phone
        .model_bytes_from(&d, &sources(vec![], true, "d"))
        .await
        .unwrap()
        .expect("downloaded");
    assert_eq!(&got[..], &body_d[..]);
    assert_eq!(server_d.requests().len(), 1);
    assert!(*progress.lock().unwrap() > 0, "progress was reported");
    assert!(
        phone
            .node()
            .blobs()
            .has(&shoebox_core::ids::Blake3Hash(d.blake3))
            .await
            .unwrap(),
        "kept as search data"
    );
    wait_until(Duration::from_secs(30), || async {
        keeper.model_offered(&d).await.unwrap()
    })
    .await
    .expect("the phone offers what it downloaded to the keeper");
    // Asked again, it is already here: no second download.
    phone
        .model_bytes_from(&d, &sources(vec![], true, "d"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(server_d.requests().len(), 1);
    c.shutdown().await;
}
