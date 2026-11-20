//! Captura de fotogramas desde la webcam vía V4L2 (sin dependencias nativas).
//!
//! Se negocia MJPG a 640x480 y se decodifica a `RgbImage` con el crate `image`.

use image::RgbImage;
use linuxvideo::format::{PixFormat, PixelFormat};
use linuxvideo::stream::ReadStream;
use linuxvideo::Device;
use std::fmt;

pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 480;

/// Fotogramas iniciales que se descartan para que la cámara ajuste exposición.
const WARMUP_FRAMES: usize = 5;

#[derive(Debug)]
pub enum CameraError {
    Open(std::io::Error),
    Capture(std::io::Error),
    Decode(image::ImageError),
}

impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CameraError::Open(e) => write!(f, "no se pudo abrir la cámara: {e}"),
            CameraError::Capture(e) => write!(f, "error al capturar fotograma: {e}"),
            CameraError::Decode(e) => write!(f, "error al decodificar MJPG: {e}"),
        }
    }
}

impl std::error::Error for CameraError {}

pub struct Camera {
    stream: ReadStream,
}

impl Camera {
    /// Abre el dispositivo y arranca el flujo de captura.
    pub fn open(device: &str) -> Result<Self, CameraError> {
        let dev = Device::open(device).map_err(CameraError::Open)?;
        let cap = dev
            .video_capture(PixFormat::new(WIDTH, HEIGHT, PixelFormat::MJPG))
            .map_err(CameraError::Open)?;
        if cap.format().pixel_format() != PixelFormat::MJPG {
            return Err(CameraError::Open(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "la cámara no soporta MJPG",
            )));
        }
        let mut stream = cap.into_stream().map_err(CameraError::Open)?;
        for _ in 0..WARMUP_FRAMES {
            stream.dequeue(|_| Ok(())).map_err(CameraError::Capture)?;
        }
        Ok(Self { stream })
    }

    /// Captura un fotograma y lo devuelve decodificado en RGB.
    pub fn grab(&mut self) -> Result<RgbImage, CameraError> {
        let mut jpeg = Vec::new();
        self.stream
            .dequeue(|buf| {
                jpeg.extend_from_slice(&buf);
                Ok(())
            })
            .map_err(CameraError::Capture)?;
        let img = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)
            .map_err(CameraError::Decode)?;
        Ok(img.to_rgb8())
    }
}
