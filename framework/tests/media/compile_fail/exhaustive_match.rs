//! IMG-008: `Transformation` is `#[non_exhaustive]`, so a match outside the
//! crate that names every variant and no wildcard does not compile.

use suprnova::Transformation;

fn name(step: Transformation) -> &'static str {
    match step {
        Transformation::Resize { .. } => "resize",
        Transformation::ResizeWidth(_) => "resize_width",
        Transformation::ResizeHeight(_) => "resize_height",
        Transformation::Scale { .. } => "scale",
        Transformation::ScaleWidth(_) => "scale_width",
        Transformation::ScaleHeight(_) => "scale_height",
        Transformation::Crop { .. } => "crop",
        Transformation::Cover { .. } => "cover",
        Transformation::Contain { .. } => "contain",
        Transformation::Rotate { .. } => "rotate",
        Transformation::FlipVertically => "flip",
        Transformation::FlipHorizontally => "flop",
        Transformation::Blur(_) => "blur",
        Transformation::Sharpen(_) => "sharpen",
        Transformation::Grayscale => "grayscale",
        Transformation::Orient => "orient",
        Transformation::Custom(_) => "custom",
    }
}

fn main() {
    let _ = name(Transformation::Grayscale);
}
