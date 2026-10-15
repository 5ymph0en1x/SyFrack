use crate::params::{join, validate_params, visit_rgb, Bounds, Params, KELVIN};
use crate::scene::SceneError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    /// Linear RGB offset added in this zone.
    pub offset: [f32; 3],
    /// Multiplicative gain in this zone: 0 = unchanged, 1 = doubled.
    pub gain: f32,
}

impl Zone {
    pub fn neutral() -> Self {
        Self {
            offset: [0.0; 3],
            gain: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Tonemap {
    /// Display-referred artwork: preserve palette channels instead of a filmic curve.
    Direct,
    /// AgX filmic curve. The look is `pow(v * slope + offset, power)`, applied
    /// after the AgX sigmoid: slope lifts, power is contrast, offset shifts black.
    Agx {
        slope: f32,
        power: f32,
        offset: f32,
    },
    Reinhard {
        white: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vignette {
    /// Radius (in half-frame units, 1 = reaches the short edge) where darkening starts.
    pub radius: f32,
    pub softness: f32,
    pub intensity: f32,
    /// Elliptical follows the frame aspect; otherwise the falloff is circular.
    pub elliptical: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grain {
    pub amount: f32,
    /// Grain cell size in pixels.
    pub size: f32,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputSpace {
    Srgb,
    DisplayP3,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Look {
    pub exposure_ev: f32,
    /// If set, exposure is computed to hit this geometric-mean luminance
    /// (0.18 is the classic mid grey) and `exposure_ev` is added on top.
    pub auto_exposure: Option<f32>,
    /// Kelvin. 6500 is neutral; lower makes the image warmer.
    pub temperature: f32,
    /// -1..1, positive is magenta, negative is green.
    pub tint: f32,
    pub shadows: Zone,
    pub midtones: Zone,
    pub highlights: Zone,
    pub tonemap: Tonemap,
    pub saturation: f32,
    pub vibrance: f32,
    #[serde(default)]
    pub neon_softness: f32,
    pub vignette: Option<Vignette>,
    pub grain: Option<Grain>,
    pub output: OutputSpace,
    pub dither: bool,
}

impl Look {
    /// Checks every look parameter against its valid range.
    pub fn validate(&self) -> Result<(), SceneError> {
        validate_params(self, "look")
    }

    /// Reads a RON look file and validates it.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Look, SceneError> {
        let text = std::fs::read_to_string(path)?;
        let look: Look = ron::from_str(&text).map_err(|e| SceneError::Parse(e.to_string()))?;
        look.validate()?;
        Ok(look)
    }

    pub fn neutral() -> Self {
        Self {
            exposure_ev: 0.0,
            auto_exposure: None,
            temperature: 6500.0,
            tint: 0.0,
            shadows: Zone::neutral(),
            midtones: Zone::neutral(),
            highlights: Zone::neutral(),
            tonemap: Tonemap::Agx {
                slope: 1.0,
                power: 1.0,
                offset: 0.0,
            },
            saturation: 1.0,
            vibrance: 0.0,
            neon_softness: 0.0,
            vignette: None,
            grain: None,
            output: OutputSpace::Srgb,
            dither: true,
        }
    }
}

const EV: Bounds = Bounds::new(-10.0, 10.0, &[(-2.0, 2.0)]);
const TINT: Bounds = Bounds::new(-1.0, 1.0, &[(-0.2, 0.2)]);
const OFFSET: Bounds = Bounds::new(-1.0, 1.0, &[(-0.1, 0.1)]);
const GAIN: Bounds = Bounds::new(-1.0, 4.0, &[(-0.3, 0.3)]);
const SLOPE: Bounds = Bounds::new(0.1, 3.0, &[(0.8, 1.3)]);
const POWER: Bounds = Bounds::new(0.1, 3.0, &[(0.9, 1.5)]);
const AGX_OFFSET: Bounds = Bounds::new(-0.5, 0.5, &[(-0.05, 0.05)]);
const WHITE: Bounds = Bounds::new(0.1, 100.0, &[(2.0, 10.0)]);
const SATURATION: Bounds = Bounds::new(0.0, 3.0, &[(0.8, 1.3)]);
const VIBRANCE: Bounds = Bounds::new(-1.0, 1.0, &[(0.0, 0.5)]);
const VRADIUS: Bounds = Bounds::new(0.0, 2.0, &[(0.6, 1.2)]);
const VSOFT: Bounds = Bounds::new(0.01, 2.0, &[(0.3, 1.0)]);
const VINTENSITY: Bounds = Bounds::new(0.0, 1.0, &[(0.0, 0.5)]);
const GRAIN_AMOUNT: Bounds = Bounds::new(0.0, 1.0, &[(0.0, 0.15)]);
const AUTO_EXPOSURE: Bounds = Bounds::new(0.01, 1.0, &[(0.1, 0.3)]);
const GRAIN_SIZE: Bounds = Bounds::new(0.5, 8.0, &[(1.0, 2.0)]);

impl Params for Zone {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        visit_rgb(&join(prefix, "offset"), &mut self.offset, OFFSET, f);
        f(&join(prefix, "gain"), &mut self.gain, GAIN);
    }
}

impl Params for Look {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        f(&join(prefix, "exposure_ev"), &mut self.exposure_ev, EV);
        if let Some(a) = &mut self.auto_exposure {
            f(&join(prefix, "auto_exposure"), a, AUTO_EXPOSURE);
        }
        f(&join(prefix, "temperature"), &mut self.temperature, KELVIN);
        f(&join(prefix, "tint"), &mut self.tint, TINT);
        self.shadows.visit_params(&join(prefix, "shadows"), f);
        self.midtones.visit_params(&join(prefix, "midtones"), f);
        self.highlights.visit_params(&join(prefix, "highlights"), f);
        let t = join(prefix, "tonemap");
        match &mut self.tonemap {
            Tonemap::Agx {
                slope,
                power,
                offset,
            } => {
                f(&join(&t, "agx.slope"), slope, SLOPE);
                f(&join(&t, "agx.power"), power, POWER);
                f(&join(&t, "agx.offset"), offset, AGX_OFFSET);
            }
            Tonemap::Reinhard { white } => f(&join(&t, "reinhard.white"), white, WHITE),
            Tonemap::Direct => {}
        }
        f(
            &join(prefix, "saturation"),
            &mut self.saturation,
            SATURATION,
        );
        f(&join(prefix, "vibrance"), &mut self.vibrance, VIBRANCE);
        f(
            &join(prefix, "neon_softness"),
            &mut self.neon_softness,
            Bounds::new(0.0, 1.0, &[(0.0, 1.0)]),
        );
        if let Some(v) = &mut self.vignette {
            let p = join(prefix, "vignette");
            f(&join(&p, "radius"), &mut v.radius, VRADIUS);
            f(&join(&p, "softness"), &mut v.softness, VSOFT);
            f(&join(&p, "intensity"), &mut v.intensity, VINTENSITY);
        }
        if let Some(g) = &mut self.grain {
            let p = join(prefix, "grain");
            f(&join(&p, "amount"), &mut g.amount, GRAIN_AMOUNT);
            f(&join(&p, "size"), &mut g.size, GRAIN_SIZE);
        }
    }
}
