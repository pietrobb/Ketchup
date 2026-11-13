//! The thread that decodes pictures (review 2026-11-13 R1113 P3): the pictures of a
//! closed document were still decoded one after another, and a panic in one stopped the
//! thread, so every picture after it showed plain.

use super::{DerivedRenderCache, PictureJob, RenderPicture, decode_pictures};
use crate::KetchupApp;
use ketchup_model::document::MeshImage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

fn job(blob: &str) -> PictureJob {
    let image = MeshImage {
        blob: blob.to_owned(),
        mime_type: "image/png".to_owned(),
    };
    (image, None)
}

/// The jobs `blobs`, all queued, and nothing more to come.
fn queued(blobs: &[&str]) -> mpsc::Receiver<PictureJob> {
    let (queue, jobs) = mpsc::channel();
    for blob in blobs {
        queue.send(job(blob)).unwrap();
    }
    jobs
}

#[test]
fn a_picture_that_panics_shows_plain_and_the_next_still_decodes() {
    let (sender, decoded) = mpsc::channel();
    decode_pictures(
        &queued(&["broken", "fine"]),
        &sender,
        &AtomicBool::new(false),
        |image, _| {
            assert_ne!(image.blob, "broken", "a decoder bug");
            Some(Arc::new(RenderPicture {
                blob: image.blob.clone(),
                size: [1, 1],
                rgba: vec![0; 4],
            }))
        },
    );
    let delivered: Vec<_> = decoded
        .try_iter()
        .map(|(blob, picture)| (blob, picture.is_some()))
        .collect();
    assert_eq!(
        delivered,
        [("broken".to_owned(), false), ("fine".to_owned(), true)]
    );
}

#[test]
fn a_cancelled_queue_decodes_nothing_more() {
    let (sender, decoded) = mpsc::channel();
    decode_pictures(
        &queued(&["a", "b", "c"]),
        &sender,
        &AtomicBool::new(true),
        |image, _| panic!("{} decoded after its document closed", image.blob),
    );
    assert_eq!(decoded.try_iter().count(), 0);
}

#[test]
fn closing_or_replacing_the_document_cancels_its_pictures() {
    let mut cache = DerivedRenderCache::default();
    cache.start_decoder(job("a")).unwrap();
    let cancelled = Arc::clone(&cache.cancelled);
    assert!(!cancelled.load(Ordering::Relaxed));
    drop(cache);
    assert!(cancelled.load(Ordering::Relaxed), "dropped");

    // Open puts another document in place: the old one's pictures are not decoded.
    let mut app = KetchupApp::new();
    app.render.cache.start_decoder(job("b")).unwrap();
    let cancelled = Arc::clone(&app.render.cache.cancelled);
    app.reset_document_presentation();
    assert!(cancelled.load(Ordering::Relaxed), "opened another");
    assert!(!app.render.cache.decoding_pictures());
}
