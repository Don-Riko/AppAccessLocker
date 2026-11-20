//! Verificación mínima de vida (anti-foto): desafío de giro de cabeza.
//!
//! Se pide girar la cabeza hacia un lado y luego mirar al frente. Una fotografía
//! impresa o estática no puede cambiar su giro (yaw), que se estima a partir de
//! los 5 puntos faciales. Limitación conocida: no protege frente a un vídeo
//! reproducido en una pantalla.

use std::time::{Duration, Instant};

/// |yaw| mínimo para considerar que la cabeza está girada.
const TURN_MIN: f32 = 0.30;
/// |yaw| máximo para considerar que la cabeza mira al frente.
const FRONT_MAX: f32 = 0.12;
/// Tiempo máximo por paso antes de reiniciar el desafío.
const STEP_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessStep {
    /// Esperando que el usuario gire la cabeza hacia un lado.
    TurnHead,
    /// Esperando que vuelva a mirar al frente.
    FaceFront,
    /// Desafío superado.
    Passed,
}

pub struct LivenessCheck {
    step: LivenessStep,
    step_started: Instant,
}

impl Default for LivenessCheck {
    fn default() -> Self {
        Self::new()
    }
}

impl LivenessCheck {
    pub fn new() -> Self {
        Self { step: LivenessStep::TurnHead, step_started: Instant::now() }
    }

    /// Instrucción a mostrar al usuario para el paso actual.
    pub fn instruction(&self) -> &'static str {
        match self.step {
            LivenessStep::TurnHead => "Gira la cabeza hacia un lado",
            LivenessStep::FaceFront => "Ahora mira al frente",
            LivenessStep::Passed => "Verificación superada",
        }
    }

    /// Alimenta el desafío con el yaw de un rostro reconocido. Devuelve el paso resultante.
    pub fn feed(&mut self, yaw: f32) -> LivenessStep {
        if self.step != LivenessStep::Passed && self.step_started.elapsed() > STEP_TIMEOUT {
            self.reset();
        }
        match self.step {
            LivenessStep::TurnHead if yaw.abs() >= TURN_MIN => self.advance(LivenessStep::FaceFront),
            LivenessStep::FaceFront if yaw.abs() <= FRONT_MAX => self.advance(LivenessStep::Passed),
            _ => {}
        }
        self.step
    }

    pub fn reset(&mut self) {
        self.step = LivenessStep::TurnHead;
        self.step_started = Instant::now();
    }

    fn advance(&mut self, next: LivenessStep) {
        self.step = next;
        self.step_started = Instant::now();
    }
}

/// Yaw considerado "de frente" (útil para el enrolamiento).
pub fn is_frontal(yaw: f32) -> bool {
    yaw.abs() <= FRONT_MAX
}

/// Yaw considerado "girado" (útil para el enrolamiento).
pub fn is_turned(yaw: f32) -> bool {
    yaw.abs() >= TURN_MIN
}
