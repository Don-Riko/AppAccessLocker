//! Hilo de visión en segundo plano. Posee la cámara y el motor de rostros, captura
//! cada cierto intervalo y envía observaciones al hilo principal de GTK.
//!
//! El hilo no conoce la identidad del usuario: solo emite el embedding del rostro
//! detectado. Comparar con la plantilla registrada es tarea del consumidor.
//! La cámara se libera cuando el hilo termina (al soltar `VisionWorker`).

use crate::camera::Camera;
use crate::vision::liveness::FaceMetrics;
use crate::vision::FaceEngine;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Resultado de una comprobación de la cámara.
#[derive(Debug, Clone)]
pub enum Observation {
    /// Se detectó un rostro válido.
    Face { score: f32, metrics: FaceMetrics, embedding: Vec<f32> },
    /// La cámara funciona pero no hay rostro.
    NoFace,
    /// Fallo de cámara o de inferencia (se reintenta en el siguiente ciclo).
    Error(String),
}

enum Command {
    SetInterval(Duration),
    /// Libera la cámara y deja de observar hasta recibir `Resume`.
    Pause,
    Resume,
    Stop,
}

pub struct VisionWorker {
    cmd_tx: Sender<Command>,
    handle: Option<JoinHandle<()>>,
}

impl VisionWorker {
    /// Arranca el hilo. Las observaciones llegan por `sink` al bucle principal de GTK.
    pub fn spawn(
        device: String,
        models_dir: PathBuf,
        interval: Duration,
        sink: gtk4::glib::Sender<Observation>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let handle = thread::Builder::new()
            .name("aal-vision".into())
            .spawn(move || {
                let engine = match FaceEngine::load(&models_dir) {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = sink.send(Observation::Error(e.to_string()));
                        // Sin modelos no hay nada que hacer: esperamos la orden de parada.
                        while let Ok(cmd) = cmd_rx.recv() {
                            if matches!(cmd, Command::Stop) {
                                break;
                            }
                        }
                        return;
                    }
                };

                let mut interval = interval;
                let mut camera: Option<Camera> = None;
                let mut next_tick = Instant::now();
                let mut paused = false;

                loop {
                    let wait = next_tick.saturating_duration_since(Instant::now());
                    match cmd_rx.recv_timeout(wait) {
                        Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                        Ok(Command::SetInterval(d)) => {
                            interval = d;
                            next_tick = Instant::now();
                            continue;
                        }
                        Ok(Command::Pause) => {
                            paused = true;
                            camera = None; // suelta el dispositivo de inmediato
                            continue;
                        }
                        Ok(Command::Resume) => {
                            paused = false;
                            next_tick = Instant::now();
                            continue;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    next_tick = Instant::now() + interval;
                    if paused {
                        continue;
                    }

                    let obs = observe(&mut camera, &engine, &device);
                    if sink.send(obs).is_err() {
                        break; // El receptor GTK ya no existe.
                    }
                }
            })
            .expect("no se pudo crear el hilo de visión");

        Self { cmd_tx, handle: Some(handle) }
    }

    /// Libera la cámara y suspende las observaciones (para cederla a otra app).
    pub fn pause(&self) {
        let _ = self.cmd_tx.send(Command::Pause);
    }

    /// Reanuda las observaciones tras `pause`.
    pub fn resume(&self) {
        let _ = self.cmd_tx.send(Command::Resume);
    }

    /// Cambia el intervalo entre comprobaciones (aplica de inmediato).
    pub fn set_interval(&self, interval: Duration) {
        let _ = self.cmd_tx.send(Command::SetInterval(interval));
    }
}

impl Drop for VisionWorker {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Command::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn observe(camera: &mut Option<Camera>, engine: &FaceEngine, device: &str) -> Observation {
    if camera.is_none() {
        match Camera::open(device) {
            Ok(c) => *camera = Some(c),
            Err(e) => return Observation::Error(e.to_string()),
        }
    }
    let cam = camera.as_mut().expect("cámara abierta");
    let img = match cam.grab() {
        Ok(img) => img,
        Err(e) => {
            *camera = None; // Forzar reapertura en el siguiente ciclo.
            return Observation::Error(e.to_string());
        }
    };
    match engine.detect(&img) {
        Ok(Some(face)) => match engine.embed(&img, &face) {
            Ok(embedding) => Observation::Face { score: face.score, metrics: face.metrics(), embedding },
            Err(e) => Observation::Error(e.to_string()),
        },
        Ok(None) => Observation::NoFace,
        Err(e) => Observation::Error(e.to_string()),
    }
}
