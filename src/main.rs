// MJPEG camera streamer
// Copyright (c) 2025 [David Ron - rd789x@gmail.com]

use axum::{
    routing::get,
    Router,
    response::Response,
    http::{HeaderValue, HeaderName, header::{CACHE_CONTROL, PRAGMA, EXPIRES, CONTENT_TYPE, CONNECTION}},
    body::Body,
};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::net::{TcpListener as StdTcpListener, SocketAddr};
use futures::stream::unfold;
use tokio::sync::{watch, RwLock};
use tokio::net::TcpListener;
use bytes::{Bytes, BytesMut, BufMut};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use gstreamer::{Caps, Fraction};
use std::time::{Duration, Instant};
use tokio::signal;
use std::fmt::Write as FmtWrite;
use std::collections::HashMap;
use socket2::{Socket, Domain, Type, Protocol};

const BUF_CAPACITY: usize = 1280 * 960 * 3 * 6 / 5;
const IDLE_TIMEOUT_MS: u64 = 5000;
const METRICS_BATCH: u64 = 10;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	// Initialize tracing subscriber for structured logging
	tracing_subscriber::fmt()
		.with_max_level(tracing::Level::WARN)
		.with_target(false)
		.with_thread_ids(false)
		.with_thread_names(false)
		.with_timer(tracing_subscriber::fmt::time::ChronoLocal::rfc_3339())
		.init();

    gst::init()?;

    let tx: Arc<watch::Sender<Option<Bytes>>> = Arc::new(watch::channel::<Option<Bytes>>(None).0);

    let pipeline_started = Arc::new(AtomicBool::new(false));
    let metrics: Arc<RwLock<HashMap<String, u64>>> = Arc::new(RwLock::new(HashMap::from([
        ("dropped_frames".to_string(), 0),
        ("frame_count".to_string(), 0),
    ])));

    // Graceful shutdown flag
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    {
        let shutdown_flag = shutdown_flag.clone();
        tokio::spawn(async move {
            let _ = signal::ctrl_c().await;
            tracing::info!("Shutting down...");
            shutdown_flag.store(true, Ordering::Relaxed);
        });
    }

    let app = Router::new()
        .route("/stream", get({
            let tx = tx.clone();
            let metrics = metrics.clone();
            let pipeline_started = pipeline_started.clone();
            let shutdown_flag = shutdown_flag.clone();
            move || mjpeg_handler(tx.clone(), pipeline_started.clone(), metrics.clone(), shutdown_flag.clone())
        }))
        .route("/metrics", get({
            let metrics = metrics.clone();
            let tx = tx.clone();
            move || metrics_handler(metrics.clone(), tx.clone())
        }))
		.route("/", get(index_handler));

	tracing::info!("MJPEG server listening on http://0.0.0.0:8080");
	tracing::info!("Stream: http://0.0.0.0:8080/stream");
	tracing::info!("Metrics: http://0.0.0.0:8080/metrics");

    // Create socket with socket2 to set TCP options
    let addr: SocketAddr = "0.0.0.0:8080".parse()?;
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    
    // Set socket options
    socket.set_tcp_nodelay(true)?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    
    // Convert to std::net::TcpListener then to tokio::net::TcpListener
    let std_listener: StdTcpListener = socket.into();
    let listener = TcpListener::from_std(std_listener)?;

    tokio::select! {
        _ = axum::serve(listener, app) => {},
        _ = async { while !shutdown_flag.load(Ordering::Relaxed) { tokio::time::sleep(Duration::from_millis(100)).await } } => {},
    }

    Ok(())
}

async fn mjpeg_handler(
    tx: Arc<watch::Sender<Option<Bytes>>>,
    pipeline_started: Arc<AtomicBool>,
    metrics: Arc<RwLock<HashMap<String, u64>>>,
    shutdown_flag: Arc<AtomicBool>,
) -> Response {
    if !pipeline_started.load(Ordering::Relaxed) {
        let tx_clone = tx.clone();
        let metrics_clone = metrics.clone();
        let shutdown_clone = shutdown_flag.clone();
        pipeline_started.store(true, Ordering::Relaxed);

        tokio::task::spawn_blocking(move || start_pipeline(tx_clone, metrics_clone, shutdown_clone));
    }

	let rx = tx.subscribe();
	let body_buf = BytesMut::with_capacity(BUF_CAPACITY);
	let body_stream = unfold(rx, move |mut rx| {
		let mut value = body_buf.clone();
		async move {
			rx.changed().await.ok()?;
			
			let chunk = {
				let borrowed = rx.borrow();
				let jpeg = borrowed.as_ref()?;
				build_chunk(jpeg, &mut value)
			};
		
			Some((Ok::<_, std::io::Error>(chunk), rx))
		}
	});

	let mut response = Response::new(Body::from_stream(body_stream));
	response.headers_mut().insert(
		CONTENT_TYPE,
		HeaderValue::from_static("multipart/x-mixed-replace; boundary=FRAME"),
	);
	response.headers_mut().insert(
		CACHE_CONTROL,
		HeaderValue::from_static("no-store, no-cache, must-revalidate"),
	);
	response.headers_mut().insert(
		PRAGMA,
		HeaderValue::from_static("no-cache"),
	);
	response.headers_mut().insert(
		EXPIRES,
		HeaderValue::from_static("0"),
	);
	response.headers_mut().insert(
		HeaderName::from_static("x-accel-buffering"),
		HeaderValue::from_static("no"),
	);
	response.headers_mut().insert(
		CONNECTION,
		HeaderValue::from_static("keep-alive"),
	);
	response
}

fn start_pipeline(
    tx: Arc<watch::Sender<Option<Bytes>>>,
    metrics: Arc<RwLock<HashMap<String, u64>>>,
    shutdown_flag: Arc<AtomicBool>,
) {
    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            break;
        }

        let source = gst::ElementFactory::make("v4l2src")
            .property("device", "/dev/video8")
            .build()
            .unwrap();

        let capsfilter = gst::ElementFactory::make("capsfilter")
            .build()
            .unwrap();
        let caps = Caps::builder("image/jpeg")
            .field("width", 1280)
            .field("height", 960)
            .field("framerate", Fraction::new(30, 1))
            .build();
        capsfilter.set_property("caps", &caps);

        let appsink = gst::ElementFactory::make("appsink")
            .build().unwrap()
            .dynamic_cast::<AppSink>().unwrap();
        appsink.set_property("sync", &false);
        appsink.set_property("emit-signals", &false);
        appsink.set_property("max-buffers", &1u32);
        appsink.set_property("drop", &true);
        appsink.set_caps(Some(&caps));

        let pipeline = gst::Pipeline::new();
        pipeline.add_many(&[&source, &capsfilter, appsink.upcast_ref::<gst::Element>()]).unwrap();
        gst::Element::link_many(&[&source, &capsfilter, appsink.upcast_ref::<gst::Element>()]).unwrap();

        match pipeline.set_state(gst::State::Playing) {
            Ok(_) => {
                tracing::info!("Camera pipeline started");
            }
            Err(_) => {
                if let Some(bus) = pipeline.bus() {
                    while let Some(msg) = bus.pop() {
                        if let gst::MessageView::Error(err) = msg.view() {
                            tracing::error!("Pipeline error: {}", err.error());
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        }

        let bus = pipeline.bus().unwrap();
        let mut buf = BytesMut::with_capacity(BUF_CAPACITY);
        let mut dropped_frames = 0u64;
        let mut frame_count = 0u64;
        let mut idle_start: Option<Instant> = None;
        let pipeline_running = true;

        while pipeline_running && !shutdown_flag.load(Ordering::Relaxed) {
            while let Some(msg) = bus.timed_pop_filtered(
                gst::ClockTime::from_mseconds(10),
                &[gst::MessageType::Error, gst::MessageType::Eos],
            ) {
                if let gst::MessageView::Error(err) = msg.view() {
                    tracing::error!("GStreamer error: {}", err.error());
                    pipeline.set_state(gst::State::Null).ok();
                    break;
                }
            }

            if tx.receiver_count() == 0 {
                if idle_start.is_none() { idle_start = Some(Instant::now()); }
                else if idle_start.unwrap().elapsed().as_millis() as u64 > IDLE_TIMEOUT_MS { 
                    break; 
                }
                tokio::task::block_in_place(|| std::thread::sleep(Duration::from_millis(50)));
                continue;
            } else { idle_start = None; }

            if let Some(sample) = appsink.try_pull_sample(gst::ClockTime::from_mseconds(10)) {
                if let Some(buffer) = sample.buffer() {
                    if let Ok(map) = buffer.map_readable() {
                        buf.clear();
                        buf.put_slice(map.as_slice());
                        let bytes = buf.split().freeze();
                        if tx.send(Some(bytes)).is_err() { dropped_frames += 1; }
                        frame_count += 1;

                        if frame_count % METRICS_BATCH == 0 {
                            let mut m = metrics.blocking_write();
                            m.insert("dropped_frames".to_string(), dropped_frames);
                            m.insert("frame_count".to_string(), frame_count);
                        }
                    }
                }
            }
        }

        pipeline.set_state(gst::State::Null).ok();
        if shutdown_flag.load(Ordering::Relaxed) {
            break;
        }
        tracing::warn!("Pipeline stopped, restarting...");
        tokio::task::block_in_place(|| std::thread::sleep(Duration::from_secs(1)));
    }
}


fn build_chunk(jpeg: &Bytes, buf: &mut BytesMut) -> Bytes {
    buf.clear();
    write!(buf, "--FRAME\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", jpeg.len()).unwrap();
    buf.put_slice(jpeg);
    buf.put_slice(b"\r\n");
    buf.split().freeze()
}

async fn metrics_handler(metrics: Arc<RwLock<HashMap<String, u64>>>, tx: Arc<watch::Sender<Option<Bytes>>>) -> Response {
    let m = metrics.read().await;
    let body = format!(
        "clients: {}\nframe_count: {}\ndropped_frames: {}\n",
        tx.receiver_count(),
        m.get("frame_count").unwrap_or(&0),
        m.get("dropped_frames").unwrap_or(&0)
    );
    Response::new(Body::from(body))
}

async fn index_handler() -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(CACHE_CONTROL, "no-store, no-cache, must-revalidate")
        .header(PRAGMA, "no-cache")
        .header(EXPIRES, "0")
        .body(Body::from(include_str!("index.html")))
        .unwrap()
}
