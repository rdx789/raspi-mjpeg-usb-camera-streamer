# MJPEG Camera Streamer

A high-performance MJPEG streaming server for USB cameras using Rust, GStreamer, and Axum.
(created by "Claude")

## Author

David Ron  
(send remarks to rd789x@gmail.com)

## Features

- Real-time MJPEG streaming from USB cameras (v4l2)
- Low-latency streaming with configurable buffer management
- Automatic pipeline restart on errors
- Metrics endpoint for monitoring
- Graceful shutdown handling
- TCP socket optimization (nodelay, reuse address)

## Requirements

- Rust 1.70+ (install from [rustup.rs](https://rustup.rs/))
- GStreamer 1.0 development libraries
- USB camera (v4l2 compatible)

### Installing GStreamer on Raspberry Pi/Debian:
```bash
sudo apt-get update
sudo apt-get install -y \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly \
    gstreamer1.0-libav \
    gstreamer1.0-tools
```

## Configuration

Edit `src/main.rs` and update the camera device path:
```rust
let source = gst::ElementFactory::make("v4l2src")
    .property("device", "/dev/video8")  // Change to your camera device
    .build()
    .unwrap();
```

To find your camera device:
```bash
ls -l /dev/video*
v4l2-ctl --list-devices
```

## Building
```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release
```

## Running
```bash
# Run debug version
cargo run

# Run release version
cargo run --release

# Or run the binary directly
./target/release/myCameraStream
```

The server will start on `http://0.0.0.0:8080`

## Endpoints

- **`/`** - Index page (requires `src/index.html`)
- **`/stream`** - MJPEG video stream
- **`/metrics`** - Server metrics (clients, frame count, dropped frames)

## Example Usage

Open in browser:
```
http://your-raspberry-pi-ip:8080/stream
```

Embed in HTML:
```html
<img src="http://your-raspberry-pi-ip:8080/stream" />
```

View metrics:
```
http://your-raspberry-pi-ip:8080/metrics
```

## Camera Settings

Default settings (1280x960 @ 30fps):
```rust
let caps = Caps::builder("image/jpeg")
    .field("width", 1280)
    .field("height", 960)
    .field("framerate", Fraction::new(30, 1))
    .build();
```

To check supported formats:
```bash
v4l2-ctl -d /dev/video8 --list-formats-ext
```

## Systemd Service

Create `/etc/systemd/system/camera-stream.service`:
```ini
[Unit]
Description=MJPEG Camera Streaming Service
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
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable camera-stream
sudo systemctl start camera-stream
sudo systemctl status camera-stream
```

## Troubleshooting

### Camera not found
```bash
# List video devices
ls -l /dev/video*

# Check permissions
groups | grep video

# Add user to video group if needed
sudo usermod -a -G video $USER
```

### Pipeline errors
Check supported formats match your configuration:
```bash
v4l2-ctl -d /dev/video8 --list-formats-ext
```

## License

MIT
