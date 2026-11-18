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
        }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }

    pub fn update(&mut self, input: &FlyInput, dt: f32) {
        let dt = dt.min(0.1); // don't jump after a stall

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
        let target = wish * speed;
        let rate = if wish.length_squared() > 0.0 { ACCEL } else { GLIDE };
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
