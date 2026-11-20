//! Verificación de vida (anti-foto / anti-vídeo).
//!
//! Flujo:
//! 1. **Identidad frontal**: el usuario registrado debe verse de frente en varias
//!    lecturas consecutivas. Es la validación principal.
//! 2. **Candado de confirmación** (solo si la política lo requiere): se lanza una
//!    instrucción *aleatoria* en texto (girar la cabeza a un lado concreto,
//!    inclinarla, acercarse) que debe cumplirse en unos segundos y después
//!    volver a la posición neutra. Una fotografía no puede moverse y un vídeo
//!    grabado no puede acatar una orden imprevisible en el momento.
//!
//! Las métricas salen de los 5 puntos del detector (ojos, nariz, comisuras).
//! Con ellos no se pueden medir de forma fiable parpadeo, guiño ni gestos de
//! boca (la anchura entre comisuras varía tanto por ruido como por una sonrisa
//! natural), así que esos retos requieren un modelo de landmarks denso.

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Cuándo exigir el candado de confirmación tras reconocer el rostro frontal.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LivenessPolicy {
    /// Siempre.
    Always,
    /// Si la similitud no es alta, o en una comprobación aleatoria (1 de cada 4).
    #[default]
    Uncertain,
    /// Nunca: basta con reconocer el rostro frontal.
    Never,
}

/// Métricas geométricas de un rostro, derivadas de los 5 puntos y la caja.
#[derive(Debug, Clone, Copy)]
pub struct FaceMetrics {
    /// Giro horizontal: desplazamiento de la nariz respecto al centro de los ojos,
    /// normalizado por la distancia interocular. ~0 de frente. Positivo cuando el
    /// usuario gira hacia *su* izquierda (la nariz se mueve a la derecha de la imagen).
    pub yaw: f32,
    /// Inclinación lateral de la cabeza, en grados (línea entre ojos).
    pub roll_deg: f32,
    /// Anchura de boca / distancia interocular. Aumenta al sonreír.
    pub mouth_ratio: f32,
    /// Ancho del rostro en píxeles. Aumenta al acercarse.
    pub face_width: f32,
}

/// Umbrales de pose.
const FRONT_YAW_MAX: f32 = 0.12;
const FRONT_ROLL_MAX: f32 = 10.0;
const TURN_YAW_MIN: f32 = 0.30;
const TILT_ROLL_MIN: f32 = 15.0;
/// Incremento relativo del ancho del rostro sobre la línea base neutra.
const CLOSER_GAIN_MIN: f32 = 0.20;

/// Lecturas frontales reconocidas consecutivas para validar la identidad.
const FRONTAL_HITS: usize = 3;
/// Lecturas no reconocidas consecutivas que anulan la verificación en curso.
const UNRECOGNIZED_TOLERANCE: usize = 3;
/// Tiempo máximo para cumplir una instrucción.
const CHALLENGE_TIMEOUT: Duration = Duration::from_secs(12);
/// Instrucciones fallidas antes de rendirse (y remitir al PIN).
const MAX_CHALLENGES: usize = 3;
/// Probabilidad (1/N) de comprobación aleatoria con política `Uncertain`.
const SPOT_CHECK_ONE_IN: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Challenge {
    TurnLeft,
    TurnRight,
    TiltHead,
    ComeCloser,
}

impl Challenge {
    const ALL: [Challenge; 4] = [
        Challenge::TurnLeft,
        Challenge::TurnRight,
        Challenge::TiltHead,
        Challenge::ComeCloser,
    ];

    fn random(exclude: Option<Challenge>) -> Challenge {
        loop {
            let c = Self::ALL[(OsRng.next_u32() % Self::ALL.len() as u32) as usize];
            if Some(c) != exclude {
                return c;
            }
        }
    }

    fn instruction(self) -> &'static str {
        match self {
            Challenge::TurnLeft => "Gira la cabeza hacia TU IZQUIERDA",
            Challenge::TurnRight => "Gira la cabeza hacia TU DERECHA",
            Challenge::TiltHead => "Inclina la cabeza hacia un hombro",
            Challenge::ComeCloser => "Acércate a la cámara",
        }
    }

    /// Instrucción para volver a la posición neutra tras cumplir el gesto.
    fn return_instruction(self) -> &'static str {
        match self {
            Challenge::ComeCloser => "Bien. Ahora vuelve a tu distancia normal",
            _ => "Bien. Ahora vuelve a mirar al frente",
        }
    }

    fn satisfied(self, m: &FaceMetrics, base: &FaceMetrics) -> bool {
        match self {
            Challenge::TurnLeft => m.yaw >= TURN_YAW_MIN,
            Challenge::TurnRight => m.yaw <= -TURN_YAW_MIN,
            Challenge::TiltHead => m.roll_deg.abs() >= TILT_ROLL_MIN,
            // Inclinar la cabeza agranda la caja: acercarse solo cuenta de frente.
            Challenge::ComeCloser => is_frontal(m) && m.face_width >= base.face_width * (1.0 + CLOSER_GAIN_MIN),
        }
    }
}

enum Stage {
    /// Acumulando lecturas frontales reconocidas.
    Frontal { hits: usize, sim_sum: f32, metrics_sum: FaceMetrics },
    /// Esperando que se cumpla la instrucción y luego se vuelva a neutro.
    Challenge { kind: Challenge, baseline: FaceMetrics, gesture_done: bool, started: Instant, attempts: usize },
    Passed,
    Failed,
}

/// Resultado de alimentar una lectura.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Sigue en curso; mostrar esta instrucción al usuario.
    Pending(String),
    /// Identidad verificada (y candado superado si se exigió).
    Passed,
    /// Se agotaron los intentos; remitir al PIN.
    Failed,
}

pub struct LivenessCheck {
    policy: LivenessPolicy,
    strong_threshold: f32,
    stage: Stage,
    unrecognized_streak: usize,
}

impl LivenessCheck {
    pub fn new(policy: LivenessPolicy, strong_threshold: f32) -> Self {
        Self { policy, strong_threshold, stage: Self::frontal_stage(), unrecognized_streak: 0 }
    }

    fn frontal_stage() -> Stage {
        Stage::Frontal {
            hits: 0,
            sim_sum: 0.0,
            metrics_sum: FaceMetrics { yaw: 0.0, roll_deg: 0.0, mouth_ratio: 0.0, face_width: 0.0 },
        }
    }

    pub fn reset(&mut self) {
        self.stage = Self::frontal_stage();
        self.unrecognized_streak = 0;
    }

    /// Alimenta una lectura: métricas del rostro, su similitud con la plantilla y
    /// si supera el umbral de reconocimiento.
    pub fn feed(&mut self, m: &FaceMetrics, similarity: f32, recognized: bool) -> Verdict {
        if matches!(self.stage, Stage::Passed) {
            return Verdict::Passed;
        }
        if matches!(self.stage, Stage::Failed) {
            return Verdict::Failed;
        }

        // Si deja de reconocerse al usuario varias veces, se reinicia la verificación.
        if recognized {
            self.unrecognized_streak = 0;
        } else {
            self.unrecognized_streak += 1;
            if self.unrecognized_streak >= UNRECOGNIZED_TOLERANCE {
                self.reset();
                return Verdict::Pending("Rostro no reconocido. Mira de frente a la cámara.".into());
            }
        }

        match &mut self.stage {
            Stage::Frontal { hits, sim_sum, metrics_sum } => {
                if !recognized {
                    return Verdict::Pending("Rostro no reconocido. Mira de frente a la cámara.".into());
                }
                if !is_frontal(m) {
                    *hits = 0;
                    *sim_sum = 0.0;
                    return Verdict::Pending("Mira de frente a la cámara".into());
                }
                *hits += 1;
                *sim_sum += similarity;
                metrics_sum.yaw += m.yaw;
                metrics_sum.roll_deg += m.roll_deg;
                metrics_sum.mouth_ratio += m.mouth_ratio;
                metrics_sum.face_width += m.face_width;
                if *hits < FRONTAL_HITS {
                    return Verdict::Pending(format!("Verificando identidad… ({}/{})", hits, FRONTAL_HITS));
                }
                let n = *hits as f32;
                let mean_sim = *sim_sum / n;
                let baseline = FaceMetrics {
                    yaw: metrics_sum.yaw / n,
                    roll_deg: metrics_sum.roll_deg / n,
                    mouth_ratio: metrics_sum.mouth_ratio / n,
                    face_width: metrics_sum.face_width / n,
                };
                if self.requires_challenge(mean_sim) {
                    let kind = Challenge::random(None);
                    eprintln!("[liveness] similitud media {mean_sim:.3}: candado requerido → {kind:?}");
                    self.stage = Stage::Challenge { kind, baseline, gesture_done: false, started: Instant::now(), attempts: 1 };
                    Verdict::Pending(kind.instruction().to_string())
                } else {
                    eprintln!("[liveness] similitud media {mean_sim:.3}: sin candado");
                    self.stage = Stage::Passed;
                    Verdict::Passed
                }
            }
            Stage::Challenge { kind, baseline, gesture_done, started, attempts } => {
                if started.elapsed() > CHALLENGE_TIMEOUT {
                    if *attempts >= MAX_CHALLENGES {
                        self.stage = Stage::Failed;
                        return Verdict::Failed;
                    }
                    *kind = Challenge::random(Some(*kind));
                    eprintln!("[liveness] tiempo agotado; nuevo candado → {kind:?} (intento {})", *attempts + 1);
                    *gesture_done = false;
                    *started = Instant::now();
                    *attempts += 1;
                    return Verdict::Pending(format!("Tiempo agotado. {}", kind.instruction()));
                }
                if !*gesture_done {
                    if kind.satisfied(m, baseline) {
                        *gesture_done = true;
                        return Verdict::Pending(kind.return_instruction().into());
                    }
                    return Verdict::Pending(kind.instruction().to_string());
                }
                // Gesto hecho: exigir vuelta a neutro con el rostro reconocido.
                if recognized && is_frontal(m) && is_neutral(m, baseline) {
                    self.stage = Stage::Passed;
                    Verdict::Passed
                } else {
                    Verdict::Pending(kind.return_instruction().into())
                }
            }
            Stage::Passed => Verdict::Passed,
            Stage::Failed => Verdict::Failed,
        }
    }

    fn requires_challenge(&self, mean_similarity: f32) -> bool {
        match self.policy {
            LivenessPolicy::Always => true,
            LivenessPolicy::Never => false,
            LivenessPolicy::Uncertain => {
                mean_similarity < self.strong_threshold || OsRng.next_u32() % SPOT_CHECK_ONE_IN == 0
            }
        }
    }
}

/// Rostro de frente (sin giro ni inclinación apreciables).
pub fn is_frontal(m: &FaceMetrics) -> bool {
    m.yaw.abs() <= FRONT_YAW_MAX && m.roll_deg.abs() <= FRONT_ROLL_MAX
}

/// Rostro girado (útil para el enrolamiento).
pub fn is_turned(m: &FaceMetrics) -> bool {
    m.yaw.abs() >= TURN_YAW_MIN
}

/// Distancia cercana a la línea base (sin acercamiento).
fn is_neutral(m: &FaceMetrics, base: &FaceMetrics) -> bool {
    m.face_width < base.face_width * (1.0 + CLOSER_GAIN_MIN / 2.0)
}
