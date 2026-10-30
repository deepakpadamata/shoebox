//! S1-17/S1-18: faces found once, grouped by one device, named anywhere; who is in a photo is
//! worked out the same way on every device, and people are found by name.

mod common;

use common::*;
use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::{BlobRef, ExifSummary, ModelId, PersonId, PhotoId, PhotoRecord, Role};
use shoebox_core::store::{FaceEdit, LibraryStore, PersonEdit};
use shoebox_node::{
    Always, FaceModels, FetcherConfig, IndexerConfig, LibraryNode, Protection, SearchOptions,
};
use shoebox_search::encoder::{normalize, EncodeError, Rgb, TestEncoder};
use shoebox_search::faces::{Detection, FaceDetector, FaceEmbedder};
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::sync::Arc;
use std::time::Duration;

const FACES: ModelId = ModelId([0xFA; 32]);

/// One face in the middle of every image.
struct OneFace;

impl FaceDetector for OneFace {
    fn detect(&self, image: &Rgb) -> Result<Vec<Detection>, EncodeError> {
        let (w, h) = (image.width as f32, image.height as f32);
        Ok(vec![Detection {
            bbox: [0.3 * w, 0.2 * h, 0.7 * w, 0.8 * h],
            landmarks: [[0.4 * w, 0.4 * h]; 5],
            score: 0.95,
        }])
    }
}

/// A face's identity is the image's colour: red is one person, blue another.
struct ByColour;

impl FaceEmbedder for ByColour {
    fn dims(&self) -> usize {
        16
    }
    fn embed(&self, image: &Rgb, _: &[[f32; 2]; 5]) -> Result<Vec<f32>, EncodeError> {
        let p = image.pixel(image.width / 2, image.height / 2);
        let mut v: Vec<f32> = (0..16)
            .map(|i| f32::from(p[i % 3]) + (i as f32) * 0.01)
            .collect();
        normalize(&mut v);
        Ok(v)
    }
}

fn jpeg(rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(200, 200, image::Rgb(rgb));
    let mut out = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&img)
        .unwrap();
    out
}

async fn add(
    n: &LibraryNode<Library>,
    others: &[&LibraryNode<Library>],
    i: u32,
    rgb: [u8; 3],
) -> PhotoId {
    let mut preview = jpeg(rgb);
    preview.extend_from_slice(&i.to_le_bytes()); // distinct blobs, still a valid JPEG
    let pv = n
        .node()
        .blobs()
        .add_bytes(preview.clone(), Protection::Library)
        .await
        .unwrap();
    for o in others {
        o.node()
            .blobs()
            .add_bytes(preview.clone(), Protection::Library)
            .await
            .unwrap();
    }
    let id = Blake3Hash::of_bytes(&[b"face".as_slice(), &i.to_le_bytes()].concat());
    let blob = |hash| BlobRef {
        hash,
        size: 1,
        mime: "image/jpeg".into(),
    };
    let h = shoebox_core::hlc::Hlc {
        physical_ms: 1,
        counter: 0,
        device: n.device(),
    };
    n.store()
        .upsert_photo(PhotoRecord {
            id,
            original: blob(id),
            preview: blob(pv),
            thumb: blob(pv),
            taken_at: Timestamp(i64::from(i)),
            exif: ExifSummary::default(),
            imported_by: n.device(),
            favorite: Lww::new(false, h),
            deleted_at: Lww::new(None, h),
        })
        .await
        .unwrap();
    id
}

async fn found(n: &LibraryNode<Library>, q: &str) -> Vec<PhotoId> {
    let mut v = n
        .search(q, &SearchOptions::utc(Timestamp(0)), 0, 100)
        .await
        .unwrap()
        .hits
        .ids;
    v.sort();
    v
}

fn sorted(mut v: Vec<PhotoId>) -> Vec<PhotoId> {
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn faces_are_named_anywhere_and_found_by_name_everywhere() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let mut red = vec![];
    let mut blue = vec![];
    for i in 0..6 {
        red.push(add(&nodes[0], &[&nodes[1]], i, [220, 30, 30]).await);
        blue.push(add(&nodes[0], &[&nodes[1]], 100 + i, [30, 30, 220]).await);
    }
    let models = FaceModels {
        detector: Arc::new(OneFace),
        embedder: Arc::new(ByColour),
        model: FACES,
    };
    nodes[0].set_faces(models.clone());
    nodes[1].set_face_model(FACES);
    for n in &nodes {
        n.start_search();
        n.start_fetcher(FetcherConfig {
            concurrency: 4,
            tick: Duration::from_millis(200),
            retry: Duration::from_millis(200),
        });
    }
    nodes[0].start_indexer(
        IndexerConfig {
            tick: Duration::from_millis(200),
            shard_after: Duration::from_millis(200),
            always_on: true,
            ..Default::default()
        },
        Arc::new(TestEncoder { dims: 16 }),
        Arc::new(Always),
    );
    wait_until(Duration::from_secs(60), || async {
        nodes[0].flush_index().await.unwrap();
        nodes[1].face_inputs(FACES).await.unwrap().len() == 12
    })
    .await
    .expect("faces found on the keeper reach the phone");
    assert!(nodes[0].regroup_people(FACES, true).await.unwrap());

    // The phone names two red faces Ana; the whole red group becomes Ana on both devices.
    let ana = PersonId([1; 16]);
    nodes[1]
        .store()
        .edit_person(
            ana,
            PersonEdit {
                name: Some("Ana".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let key = LibraryNode::<Library>::face_key([0.3, 0.2, 0.7, 0.8]).0;
    for p in &red[..2] {
        nodes[1]
            .store()
            .label_face(
                *p,
                key,
                FaceEdit {
                    is: Some(Some(ana)),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    for n in &nodes {
        wait_until(Duration::from_secs(30), || async {
            found(n, "Ana").await == sorted(red.clone())
        })
        .await
        .expect("the red group is Ana everywhere");
    }

    // Offline, the phone says one red face is not Ana while the keeper groups again.
    c.take_offline(1);
    nodes[1]
        .store()
        .label_face(
            red[5],
            key,
            FaceEdit {
                is_not: Some(ana),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    add(&nodes[0], &[], 50, [220, 30, 30]).await;
    nodes[0].flush_index().await.unwrap();
    nodes[0].regroup_people(FACES, true).await.unwrap();
    c.bring_online(1);
    let want: Vec<PhotoId> = sorted(red[..5].to_vec());
    for n in &nodes[1..] {
        wait_until(Duration::from_secs(60), || async {
            let got = found(n, "Ana").await;
            want.iter().all(|p| got.contains(p)) && !got.contains(&red[5])
        })
        .await
        .expect("not-Ana wins, and the name stuck through the regrouping");
    }

    // Merging Ben into Ana: searching Ana finds both.
    let ben = PersonId([2; 16]);
    nodes[0]
        .store()
        .edit_person(
            ben,
            PersonEdit {
                name: Some("Ben".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    nodes[0]
        .store()
        .label_face(
            blue[0],
            key,
            FaceEdit {
                is: Some(Some(ben)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_until(Duration::from_secs(30), || async {
        found(&nodes[0], "Ben").await.len() == 6
    })
    .await
    .expect("Ben named");
    nodes[0]
        .store()
        .edit_person(
            ben,
            PersonEdit {
                merged_into: Some(Some(ana)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_until(Duration::from_secs(30), || async {
        found(&nodes[0], "Ana").await.len() >= 11
    })
    .await
    .expect("merged");
    // People off for the library: no names.
    nodes[0].set_people(false).await.unwrap();
    wait_until(Duration::from_secs(30), || async {
        found(&nodes[0], "Ana").await.is_empty()
    })
    .await
    .expect("people off");
    c.shutdown().await;
}
