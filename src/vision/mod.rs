//! Motor de visión: detección de rostro (SCRFD-500m), alineación por 5 puntos y
//! embedding facial (MobileFaceNet, 512-d). Inferencia con `tract-onnx`, sin
//! dependencias nativas. Modelos del paquete `buffalo_sc` de insightface.

pub mod liveness;
pub mod worker;

use image::RgbImage;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use tract_onnx::prelude::*;

type Model = Arc<TypedRunnableModel>;

const DET_FILE: &str = "det_500m.onnx";
const REC_FILE: &str = "w600k_mbf.onnx";

/// Lado de la entrada cuadrada del detector.
const DET_SIZE: usize = 320;
/// Umbral de confianza del detector.
const DET_SCORE_MIN: f32 = 0.5;
/// Ancho mínimo del rostro (px, en la imagen original) para considerarlo válido.
const MIN_FACE_PX: f32 = 60.0;
/// Strides de las tres cabezas de SCRFD; cada celda tiene 2 anclas.
const STRIDES: [usize; 3] = [8, 16, 32];
const ANCHORS_PER_CELL: usize = 2;

/// Lado de la entrada del modelo de embedding.
const REC_SIZE: u32 = 112;
pub const EMBEDDING_LEN: usize = 512;

/// Puntos de referencia ArcFace (112x112): ojo izq, ojo der, nariz, boca izq, boca der.
const ARCFACE_REF: [[f32; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];

#[derive(Debug)]
pub enum VisionError {
    ModelsMissing(String),
    Model(String),
    Inference(String),
}

impl fmt::Display for VisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VisionError::ModelsMissing(p) => write!(f, "no se encontraron los modelos en {p}"),
            VisionError::Model(e) => write!(f, "error al cargar modelo: {e}"),
            VisionError::Inference(e) => write!(f, "error de inferencia: {e}"),
        }
    }
}

impl std::error::Error for VisionError {}

/// Rostro detectado, en coordenadas de la imagen original.
#[derive(Debug, Clone, Copy)]
pub struct Face {
    pub score: f32,
    /// [x0, y0, x1, y1]
    pub bbox: [f32; 4],
    /// 5 puntos: ojo izq, ojo der, nariz, boca izq, boca der.
    pub kps: [[f32; 2]; 5],
}

impl Face {
    pub fn width(&self) -> f32 {
        self.bbox[2] - self.bbox[0]
    }

    /// Estimación de giro horizontal de la cabeza a partir de los 5 puntos:
    /// desplazamiento de la nariz respecto al punto medio de los ojos,
    /// normalizado por la distancia interocular. ~0 de frente; |yaw| crece al girar.
    pub fn yaw(&self) -> f32 {
        let (l, r, n) = (self.kps[0], self.kps[1], self.kps[2]);
        let mid_x = (l[0] + r[0]) / 2.0;
        let eye_dist = ((r[0] - l[0]).powi(2) + (r[1] - l[1]).powi(2)).sqrt().max(1.0);
        (n[0] - mid_x) / eye_dist
    }
}

pub struct FaceEngine {
    det: Model,
    rec: Model,
}

impl FaceEngine {
    /// Carga y optimiza ambos modelos desde `models_dir`.
    pub fn load(models_dir: &Path) -> Result<Self, VisionError> {
        let det_path = models_dir.join(DET_FILE);
        let rec_path = models_dir.join(REC_FILE);
        if !det_path.exists() || !rec_path.exists() {
            return Err(VisionError::ModelsMissing(models_dir.display().to_string()));
        }
        let det = tract_onnx::onnx()
            .model_for_path(&det_path)
            .and_then(|m| m.with_input_fact(0, f32::fact([1, 3, DET_SIZE, DET_SIZE]).into()))
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(|e| VisionError::Model(e.to_string()))?;
        let rec = tract_onnx::onnx()
            .model_for_path(&rec_path)
            .and_then(|m| m.with_input_fact(0, f32::fact([1, 3, REC_SIZE as usize, REC_SIZE as usize]).into()))
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(|e| VisionError::Model(e.to_string()))?;
        Ok(Self { det, rec })
    }

    /// Detecta el rostro más confiable de la imagen (o `None` si no hay ninguno).
    pub fn detect(&self, img: &RgbImage) -> Result<Option<Face>, VisionError> {
        let (w, h) = (img.width() as f32, img.height() as f32);
        let scale = (DET_SIZE as f32 / w.max(h)).min(1.0);
        let (nw, nh) = ((w * scale) as u32, (h * scale) as u32);
        let small = image::imageops::resize(img, nw, nh, image::imageops::FilterType::Triangle);

        let mut input = tract_ndarray::Array4::<f32>::zeros((1, 3, DET_SIZE, DET_SIZE));
        for (x, y, p) in small.enumerate_pixels() {
            for c in 0..3 {
                input[[0, c, y as usize, x as usize]] = (p[c] as f32 - 127.5) / 128.0;
            }
        }
        let outs = self
            .det
            .run(tvec!(input.into_tensor().into()))
            .map_err(|e| VisionError::Inference(e.to_string()))?;

        let mut best: Option<Face> = None;
        for (k, &stride) in STRIDES.iter().enumerate() {
            let scores = outs[k].to_plain_array_view::<f32>().map_err(|e| VisionError::Inference(e.to_string()))?;
            let boxes = outs[k + 3].to_plain_array_view::<f32>().map_err(|e| VisionError::Inference(e.to_string()))?;
            let kps = outs[k + 6].to_plain_array_view::<f32>().map_err(|e| VisionError::Inference(e.to_string()))?;
            let cells_w = DET_SIZE / stride;
            let st = stride as f32;
            for idx in 0..scores.shape()[0] {
                let score = scores[[idx, 0]];
                if score < DET_SCORE_MIN || best.map_or(false, |b| score <= b.score) {
                    continue;
                }
                let cell = idx / ANCHORS_PER_CELL;
                let cx = ((cell % cells_w) * stride) as f32;
                let cy = ((cell / cells_w) * stride) as f32;
                let inv = 1.0 / scale;
                let bbox = [
                    (cx - boxes[[idx, 0]] * st) * inv,
                    (cy - boxes[[idx, 1]] * st) * inv,
                    (cx + boxes[[idx, 2]] * st) * inv,
                    (cy + boxes[[idx, 3]] * st) * inv,
                ];
                let mut pts = [[0f32; 2]; 5];
                for j in 0..5 {
                    pts[j] = [(cx + kps[[idx, j * 2]] * st) * inv, (cy + kps[[idx, j * 2 + 1]] * st) * inv];
                }
                best = Some(Face { score, bbox, kps: pts });
            }
        }
        Ok(best.filter(|f| f.width() >= MIN_FACE_PX))
    }

    /// Alinea el rostro a 112x112 y devuelve su embedding L2-normalizado (512-d).
    pub fn embed(&self, img: &RgbImage, face: &Face) -> Result<Vec<f32>, VisionError> {
        let aligned = align_face(img, &face.kps);
        let mut input = tract_ndarray::Array4::<f32>::zeros((1, 3, REC_SIZE as usize, REC_SIZE as usize));
        for (x, y, p) in aligned.enumerate_pixels() {
            for c in 0..3 {
                input[[0, c, y as usize, x as usize]] = (p[c] as f32 - 127.5) / 127.5;
            }
        }
        let out = self
            .rec
            .run(tvec!(input.into_tensor().into()))
            .map_err(|e| VisionError::Inference(e.to_string()))?;
        let view = out[0].to_plain_array_view::<f32>().map_err(|e| VisionError::Inference(e.to_string()))?;
        let mut emb: Vec<f32> = view.iter().copied().collect();
        l2_normalize(&mut emb);
        Ok(emb)
    }
}

/// Transformación de similitud (escala+rotación+traslación) por mínimos cuadrados
/// que lleva `src` a `dst`. Devuelve (a, b, tx, ty) con x' = a·x − b·y + tx, y' = b·x + a·y + ty.
fn similarity_transform(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> (f32, f32, f32, f32) {
    let n = src.len() as f32;
    let (mut msx, mut msy, mut mdx, mut mdy) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..5 {
        msx += src[i][0];
        msy += src[i][1];
        mdx += dst[i][0];
        mdy += dst[i][1];
    }
    let (msx, msy, mdx, mdy) = (msx / n, msy / n, mdx / n, mdy / n);
    let (mut num_a, mut num_b, mut den) = (0.0, 0.0, 0.0);
    for i in 0..5 {
        let (sx, sy) = (src[i][0] - msx, src[i][1] - msy);
        let (dx, dy) = (dst[i][0] - mdx, dst[i][1] - mdy);
        num_a += sx * dx + sy * dy;
        num_b += sx * dy - sy * dx;
        den += sx * sx + sy * sy;
    }
    let den = den.max(1e-6);
    let (a, b) = (num_a / den, num_b / den);
    let tx = mdx - (a * msx - b * msy);
    let ty = mdy - (b * msx + a * msy);
    (a, b, tx, ty)
}

/// Recorta y alinea el rostro a 112x112 usando los 5 puntos y los de referencia ArcFace.
fn align_face(img: &RgbImage, kps: &[[f32; 2]; 5]) -> RgbImage {
    let (a, b, tx, ty) = similarity_transform(kps, &ARCFACE_REF);
    let det = (a * a + b * b).max(1e-6);
    let mut out = RgbImage::new(REC_SIZE, REC_SIZE);
    for v in 0..REC_SIZE {
        for u in 0..REC_SIZE {
            // Inversa: de destino (u,v) a origen (x,y).
            let (du, dv) = (u as f32 - tx, v as f32 - ty);
            let x = (a * du + b * dv) / det;
            let y = (-b * du + a * dv) / det;
            out.put_pixel(u, v, sample_bilinear(img, x, y));
        }
    }
    out
}

fn sample_bilinear(img: &RgbImage, x: f32, y: f32) -> image::Rgb<u8> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |xi: i64, yi: i64| -> [f32; 3] {
        if xi < 0 || yi < 0 || xi >= w || yi >= h {
            [0.0; 3]
        } else {
            let p = img.get_pixel(xi as u32, yi as u32);
            [p[0] as f32, p[1] as f32, p[2] as f32]
        }
    };
    let (p00, p10, p01, p11) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
    let mut out = [0u8; 3];
    for c in 0..3 {
        let top = p00[c] * (1.0 - fx) + p10[c] * fx;
        let bot = p01[c] * (1.0 - fx) + p11[c] * fx;
        out[c] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    image::Rgb(out)
}

pub fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-6 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

/// Similitud coseno entre dos embeddings ya normalizados.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Promedia varios embeddings en una plantilla normalizada.
pub fn mean_template(embeddings: &[Vec<f32>]) -> Vec<f32> {
    let mut acc = vec![0f32; EMBEDDING_LEN];
    for e in embeddings {
        for (a, x) in acc.iter_mut().zip(e) {
            *a += x;
        }
    }
    l2_normalize(&mut acc);
    acc
}
