// Free-flying camera with inertia: velocity glides, mouse look eases out,
// and the view banks into turns like a flight.

use glam::{Mat4, Quat, Vec3};

// Tuning (rates are per second; higher = snappier).
const LOOK_SENS: f32 = 0.0035; // radians per mouse pixel
const LOOK_EASE: f32 = 9.0; // how fast the view catches up with the mouse
const ACCEL: f32 = 4.0; // speeding up while a key is held
const GLIDE: f32 = 1.8; // slowing down after keys are released (low = long glide)
const BANK_PER_TURN: f32 = 0.22; // roll (rad) per rad/s of yaw rate
const BANK_PER_STRAFE: f32 = 0.12; // roll at full strafe speed
const BANK_MAX: f32 = 0.45;
const BANK_EASE: f32 = 3.5;

pub struct Camera {
    pub pos: Vec3,
    pub yaw: f32,   // 0 looks along −Z
    pub pitch: f32, // + looks up
    pub roll: f32,  // + banks right
    target_yaw: f32,
    target_pitch: f32,
    pub vel: Vec3,
    pub speed: f32, // base units / second (wheel adjusts)
    pub fov_y: f32,
    pub autopilot: Option<Autopilot>,
}

/// A flight to a viewpoint: where to be, and what to look at on arrival.
#[derive(Clone, Copy, Debug)]
pub struct Autopilot {
    pub eye: Vec3,
    pub look: Vec3,
}

#[derive(Default)]
pub struct FlyInput {
    pub forward: f32, // −1..1
    pub right: f32,
    pub up: f32,
    pub boost: bool,
    pub look_dx: f32, // pixels
    pub look_dy: f32,
}

fn ease(rate: f32, dt: f32) -> f32 {
    1.0 - (-dt * rate).exp()
}

impl Camera {
    pub fn new() -> Camera {
        Camera {
            pos: Vec3::new(0.0, 12.0, 80.0),
            yaw: 0.0,
            pitch: -0.1,
            roll: 0.0,
            target_yaw: 0.0,
            target_pitch: -0.1,
            vel: Vec3::ZERO,
            speed: 30.0,
            fov_y: 60f32.to_radians(),
            autopilot: None,
        }
    }

    /// Starts a smooth flight; any manual input takes over again.
    pub fn fly_to(&mut self, eye: Vec3, look: Vec3) {
        self.autopilot = Some(Autopilot { eye, look });
    }

    /// Jumps straight to a viewpoint (used when a new folder loads).
    pub fn place(&mut self, eye: Vec3, look: Vec3) {
        self.pos = eye;
        self.vel = Vec3::ZERO;
        self.autopilot = None;
        let (yaw, pitch) = yaw_pitch(look - eye);
        (self.yaw, self.pitch, self.target_yaw, self.target_pitch, self.roll) = (yaw, pitch, yaw, pitch, 0.0);
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }

    pub fn update(&mut self, input: &FlyInput, dt: f32) {
        let dt = dt.min(0.1); // don't jump after a stall
        let manual = input.forward != 0.0 || input.right != 0.0 || input.up != 0.0 || input.look_dx != 0.0 || input.look_dy != 0.0;
        if manual {
            self.autopilot = None;
        }
        if let Some(ap) = self.autopilot {
            // Turn towards the target (the shortest way round), and head for the eye point.
            let (yaw, pitch) = yaw_pitch(ap.look - self.pos.lerp(ap.eye, 0.5));
            let tau = std::f32::consts::TAU;
            self.target_yaw = yaw + ((self.target_yaw - yaw) / tau).round() * tau;
            self.target_pitch = pitch;
        }

        // Mouse moves a target; the view eases towards it.
        self.target_yaw += input.look_dx * LOOK_SENS;
        self.target_pitch = (self.target_pitch - input.look_dy * LOOK_SENS).clamp(-1.5, 1.5);
        let prev_yaw = self.yaw;
        let k = ease(LOOK_EASE, dt);
        self.yaw += (self.target_yaw - self.yaw) * k;
        self.pitch += (self.target_pitch - self.pitch) * k;
        let yaw_rate = if dt > 0.0 { (self.yaw - prev_yaw) / dt } else { 0.0 };

        // Velocity: quick to build up, long glide when you let go.
        let fwd = self.forward();
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let speed = self.speed * if input.boost { 4.0 } else { 1.0 };
        let wish = fwd * input.forward + right * input.right + Vec3::Y * input.up;
        let mut target = wish * speed;
        let mut rate = if wish.length_squared() > 0.0 { ACCEL } else { GLIDE };
        if let Some(ap) = self.autopilot {
            // Ease in: fast across the city, slowing as it arrives.
            let to = ap.eye - self.pos;
            let max = self.speed.max(30.0) * 3.0;
            target = (to * 1.6).clamp_length_max(max);
            rate = ACCEL;
            if to.length() < 0.25 && self.vel.length() < 0.5 {
                self.autopilot = None;
            }
        }
        self.vel += (target - self.vel) * ease(rate, dt);
        self.pos += self.vel * dt;
        if self.pos.y < 0.6 {
            // stay above the circuit board
            self.pos.y = 0.6;
            self.vel.y = self.vel.y.max(0.0);
        }

        // Bank into turns and strafes.
        let strafe = self.vel.dot(right) / self.speed.max(1e-3);
        let want = (yaw_rate * BANK_PER_TURN + strafe.clamp(-1.0, 1.0) * BANK_PER_STRAFE).clamp(-BANK_MAX, BANK_MAX);
        self.roll += (want - self.roll) * ease(BANK_EASE, dt);
    }

    /// Autofly: the flight path puts the camera at `pos` looking at `look`; the view
    /// eases round and banks into the turns as in manual flight.
    pub fn steer(&mut self, pos: Vec3, look: Vec3, dt: f32) {
        let dt = dt.clamp(1e-4, 0.1);
        self.autopilot = None;
        self.vel = (pos - self.pos) / dt; // so taking over by hand glides on
        self.pos = pos;
        let (yaw, pitch) = yaw_pitch(look - pos);
        let tau = std::f32::consts::TAU;
        self.target_yaw = yaw + ((self.target_yaw - yaw) / tau).round() * tau;
        self.target_pitch = pitch;
        let prev_yaw = self.yaw;
        let k = ease(LOOK_EASE * 0.6, dt);
        self.yaw += (self.target_yaw - self.yaw) * k;
        self.pitch += (self.target_pitch - self.pitch) * k;
        let yaw_rate = (self.yaw - prev_yaw) / dt;
        let want = (yaw_rate * BANK_PER_TURN * 1.6).clamp(-BANK_MAX, BANK_MAX);
        self.roll += (want - self.roll) * ease(BANK_EASE, dt);
    }

    pub fn view(&self) -> Mat4 {
        let fwd = self.forward().normalize();
        // Positive roll about the view axis tips "up" towards the right.
        let up = Quat::from_axis_angle(fwd, self.roll) * Vec3::Y;
        glam::camera::rh::view::look_to_mat4(self.pos, fwd, up.normalize())
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective(self.fov_y, aspect, 0.3, 4000.0)
    }

    /// World-space ray through a pixel.
    pub fn ray(&self, x: f32, y: f32, w: f32, h: f32) -> (Vec3, Vec3) {
        let inv = (self.proj(w / h) * self.view()).inverse();
        let ndc = glam::Vec2::new(x / w * 2.0 - 1.0, 1.0 - y / h * 2.0);
        let a = inv.project_point3(ndc.extend(0.0));
        let b = inv.project_point3(ndc.extend(1.0));
        (self.pos, (b - a).normalize())
    }
}

/// Yaw/pitch (in this camera's convention) that face along `d`.
fn yaw_pitch(d: Vec3) -> (f32, f32) {
    let d = d.normalize_or(Vec3::NEG_Z);
    (d.x.atan2(-d.z), d.y.clamp(-1.0, 1.0).asin().clamp(-1.5, 1.5))
}
