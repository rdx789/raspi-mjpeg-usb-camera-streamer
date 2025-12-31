# USB Camera / MJPEG Stream — Overview
(created by "Copilot")

# Author
David Ron  
(send remarks to rd789x@gmail.com)

This document describes the MJPEG server implemented in src/main.rs and provides usage, configuration, and troubleshooting information.

# Summary
- Small Rust HTTP server (Axum) that serves an MJPEG stream from a USB/v4l2 camera using GStreamer.
- Serves / (embedded index), /stream (multipart MJPEG), and /metrics (text diagnostics).
- Pipeline starts on first client, stops after idle timeout, and restarts on demand.
- Uses socket2 for TCP tuning and tracing for logging.

# Features
- Real-time MJPEG streaming from USB/v4l2 cameras
- Low-latency delivery using appsink and an in-process watch channel
- Automatic restart on pipeline errors
- Metrics endpoint (clients, frame_count, dropped_frames)
- Graceful shutdown (Ctrl+C)
- Tunable constants (buffer capacity, idle timeout, metrics batch)

# Pipeline & behavior (from src/main.rs)
- Source: v4l2src with device set to /dev/video8 by default.
- Caps: image/jpeg, width 1280, height 960, framerate 30/1 (set via capsfilter).
- Sink: appsink with properties: sync=false, emit-signals=false, max-buffers=1, drop=true.
- Frames are read from appsink, wrapped, and broadcast to connected HTTP clients via a tokio::watch channel.
- Chunk framing used for each JPEG frame:
  --FRAME\r\nContent-Type: image/jpeg\r\nContent-Length: <len>\r\n\r\n<BINARY JPEG>\r\n
- HTTP response header: Content-Type: multipart/x-mixed-replace; boundary=FRAME.
- Pipeline lifecycle:
  - Starts when first client connects to /stream.
  - If no clients connected for IDLE_TIMEOUT_MS (5000 ms), pipeline stops.
  - On errors, pipeline stops and is retried with a short backoff.
  - Shutdown flag set by Ctrl+C causes graceful exit.

# HTTP endpoints
- GET / — serves the page from src/index.html.
- GET /stream — MJPEG multipart stream for browsers or clients.
- GET /metrics — returns plain text:
  - clients: <number>
  - frame_count: <total frames produced>
  - dropped_frames: <dropped frames count>

# Networking & runtime
- Binds to 0.0.0.0:8080 (socket options: TCP_NODELAY, reuse address, non-blocking, backlog 128).
- Logging via tracing (configured in main).
- Key constants in src/main.rs:
  - BUF_CAPACITY — buffer capacity for frames
  - IDLE_TIMEOUT_MS — idle shutdown timeout
  - METRICS_BATCH — how frequently metrics are flushed

# Required GStreamer elements
- v4l2src
- capsfilter
- appsink
- JPEG (image/jpeg) support

# Install / verify (Debian / Raspberry Pi example)
```bash
sudo apt update
sudo apt install -y gstreamer1.0-tools \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly \
  gstreamer1.0-libav
gst-inspect-1.0 v4l2src capsfilter appsink

# Build & run
cargo build --release
cargo run --release
```
server listens on http://0.0.0.0:8080


# Configuration notes
Camera device path is hard-coded in src/main.rs:
Update the v4l2src device property (e.g., /dev/video0) in start_pipeline.
Adjust constants in src/main.rs to alter buffer sizes, idle timeout, or metrics batch.

# Systemd service example:
Create /etc/systemd/system/camera-stream.service (adjust paths/user):
[Unit]
Description=MJPEG Camera Stream
After=network.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/raspi_rust_usb_stream_browser
ExecStart=/home/pi/raspi_rust_usb_stream_browser/target/release/myCameraStream
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
Alias=camera.service

# Troubleshooting
Camera not found:
Verify devices: ls -l /dev/video*
Check formats: v4l2-ctl -d /dev/video8 --list-formats-ext
Check permissions: groups | grep video and sudo usermod -a -G video $USER
Pipeline errors:
Watch logs (binary prints via tracing).
Ensure required GStreamer plugins are installed and that the chosen caps are supported by the device.
