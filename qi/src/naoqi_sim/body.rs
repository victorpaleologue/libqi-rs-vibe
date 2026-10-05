//! The simulated body: joint dynamics and odometry.
//!
//! A [`Body`] holds the angles of the joints and the pose of the robot, and is ticked
//! periodically to move the joints toward their targets at the commanded fraction of their
//! maximum velocity, and to integrate the commanded velocity of the base into its pose. Every
//! tick publishes the sensor keys of the joints to the [`Memory`].

use crate::naoqi_sim::{
    lock,
    memory::Memory,
    robot::{RobotDescription, RobotModel},
};
use std::{
    fmt::Write as _,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// The frames of `ALMotion.getPosition`.
pub mod frame {
    /// The torso frame.
    pub const TORSO: i32 = 0;
    /// The world frame, fixed when the robot starts (the odometry frame).
    pub const WORLD: i32 = 1;
    /// The robot frame, on the ground below the torso.
    pub const ROBOT: i32 = 2;
}

/// The simulated joints and base of a robot. Shared between the services and the ticker.
pub struct Body {
    description: &'static RobotDescription,
    memory: Memory,
    keys: Vec<JointKeys>,
    state: Mutex<State>,
    awake: AtomicBool,
    start: Instant,
}

/// The pose of the robot in the world frame.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pose {
    /// The position along the x axis, in meters.
    pub x: f32,
    /// The position along the y axis, in meters.
    pub y: f32,
    /// The orientation around the z axis, in radians.
    pub theta: f32,
}

struct State {
    angles: Vec<f32>,
    targets: Vec<f32>,
    fractions: Vec<f32>,
    velocities: Vec<f32>,
    stiffness: Vec<f32>,
    temperatures: Vec<f32>,
    pose: Pose,
    velocity: [f32; 3],
    move_target: Option<(Instant, Pose)>,
    last_tick: Instant,
}

/// The memory keys of a joint, formatted once.
struct JointKeys {
    position_sensor: String,
    position_actuator: String,
    current: String,
    temperature: String,
    hardness: String,
    velocity: String,
    torque: String,
}

impl JointKeys {
    fn new(name: &str) -> Self {
        let device = format!("Device/SubDeviceList/{name}");
        Self {
            position_sensor: format!("{device}/Position/Sensor/Value"),
            position_actuator: format!("{device}/Position/Actuator/Value"),
            current: format!("{device}/ElectricCurrent/Sensor/Value"),
            temperature: format!("{device}/Temperature/Sensor/Value"),
            hardness: format!("{device}/Hardness/Actuator/Value"),
            velocity: format!("Motion/Velocity/Sensor/{name}"),
            torque: format!("Motion/Torque/Sensor/{name}"),
        }
    }
}

/// The nominal temperature of a joint at rest, in degrees Celsius.
const REST_TEMPERATURE: f32 = 38.0;

impl Body {
    /// Creates the body of a robot, publishing its initial sensor keys to the memory.
    pub fn new(description: &'static RobotDescription, memory: Memory) -> Arc<Self> {
        let count = description.joints.len();
        let angles: Vec<f32> = description
            .joints
            .iter()
            .map(|joint| joint.initial)
            .collect();
        let now = Instant::now();
        let body = Arc::new(Self {
            description,
            memory,
            keys: description
                .joints
                .iter()
                .map(|joint| JointKeys::new(joint.name))
                .collect(),
            state: Mutex::new(State {
                targets: angles.clone(),
                angles,
                fractions: vec![0.2; count],
                velocities: vec![0.0; count],
                stiffness: vec![1.0; count],
                temperatures: vec![REST_TEMPERATURE; count],
                pose: Pose::default(),
                velocity: [0.0; 3],
                move_target: None,
                last_tick: now,
            }),
            awake: AtomicBool::new(true),
            start: now,
        });
        body.tick();
        body
    }

    /// The description of the robot.
    pub fn description(&self) -> &'static RobotDescription {
        self.description
    }

    /// The model of the robot.
    pub fn model(&self) -> RobotModel {
        self.description.model
    }

    /// The names of the joints of the `Body` group.
    pub fn joint_names(&self) -> Vec<String> {
        self.description.body_names()
    }

    /// Resolves joint or group names to joint indexes. See [`RobotDescription::resolve`].
    pub fn resolve(&self, names: &[String]) -> Result<Vec<usize>, String> {
        self.description.resolve(names)
    }

    /// The angles of joints: the sensed angles, or the commanded ones when `use_sensors` is
    /// false.
    pub fn angles(&self, indexes: &[usize], use_sensors: bool) -> Vec<f32> {
        let state = lock(&self.state);
        let source = if use_sensors {
            &state.angles
        } else {
            &state.targets
        };
        indexes.iter().map(|&index| source[index]).collect()
    }

    /// Sets the target angles of joints, reached at the given fraction of their maximum
    /// velocity. A single angle applies to all the joints.
    pub fn set_angles(
        &self,
        indexes: &[usize],
        angles: &[f32],
        fraction: f32,
    ) -> Result<(), String> {
        let angles = broadcast(indexes.len(), angles, "angles")?;
        let fraction = check_fraction(fraction)?;
        let mut state = lock(&self.state);
        for (&index, angle) in indexes.iter().zip(angles) {
            state.targets[index] = self.description.joints[index].clamp(angle);
            state.fractions[index] = fraction;
        }
        Ok(())
    }

    /// Changes the target angles of joints by the given amounts.
    pub fn change_angles(
        &self,
        indexes: &[usize],
        deltas: &[f32],
        fraction: f32,
    ) -> Result<(), String> {
        let deltas = broadcast(indexes.len(), deltas, "angles")?;
        let fraction = check_fraction(fraction)?;
        let mut state = lock(&self.state);
        for (&index, delta) in indexes.iter().zip(deltas) {
            let target = state.targets[index] + delta;
            state.targets[index] = self.description.joints[index].clamp(target);
            state.fractions[index] = fraction;
        }
        Ok(())
    }

    /// The stiffness of joints, between 0 and 1.
    pub fn stiffness(&self, indexes: &[usize]) -> Vec<f32> {
        let state = lock(&self.state);
        indexes
            .iter()
            .map(|&index| state.stiffness[index])
            .collect()
    }

    /// Sets the stiffness of joints. A single value applies to all the joints.
    pub fn set_stiffness(&self, indexes: &[usize], values: &[f32]) -> Result<(), String> {
        let values = broadcast(indexes.len(), values, "stiffnesses")?;
        let mut state = lock(&self.state);
        for (&index, value) in indexes.iter().zip(values) {
            state.stiffness[index] = value.clamp(0.0, 1.0);
        }
        Ok(())
    }

    /// The limits of joints: minimum angle, maximum angle, maximum velocity, maximum torque.
    pub fn limits(&self, indexes: &[usize]) -> Vec<[f32; 4]> {
        indexes
            .iter()
            .map(|&index| self.description.joints[index].limits())
            .collect()
    }

    /// The temperatures of joints, in degrees Celsius.
    pub fn temperatures(&self, indexes: &[usize]) -> Vec<f32> {
        let state = lock(&self.state);
        indexes
            .iter()
            .map(|&index| state.temperatures[index])
            .collect()
    }

    /// Returns true once the sensed angles of the joints are within the tolerance of their
    /// targets.
    pub fn reached(&self, indexes: &[usize], tolerance: f32) -> bool {
        let state = lock(&self.state);
        indexes
            .iter()
            .all(|&index| (state.angles[index] - state.targets[index]).abs() <= tolerance)
    }

    /// Waits until the joints reach their targets, or the timeout expires. Returns whether
    /// the targets were reached.
    pub async fn wait_reached(&self, indexes: &[usize], tolerance: f32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.reached(indexes, tolerance) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The maximum velocities of the base: forward, lateral and rotation.
    pub fn max_velocity(&self) -> [f32; 3] {
        self.description.max_velocity
    }

    /// Sets the velocity of the base in the robot frame, clamped to the maximum velocities.
    pub fn set_velocity(&self, velocity: [f32; 3]) {
        let max = self.description.max_velocity;
        let mut state = lock(&self.state);
        state.move_target = None;
        for axis in 0..3 {
            state.velocity[axis] = velocity[axis].clamp(-max[axis], max[axis]);
        }
    }

    /// Sets the velocity of the base as fractions of the maximum velocities, in `[-1, 1]`.
    pub fn set_velocity_fraction(&self, fractions: [f32; 3]) {
        let max = self.description.max_velocity;
        let mut velocity = [0.0; 3];
        for axis in 0..3 {
            velocity[axis] = fractions[axis].clamp(-1.0, 1.0) * max[axis];
        }
        self.set_velocity(velocity);
    }

    /// The velocity of the base in the robot frame: forward, lateral and rotation.
    pub fn velocity(&self) -> [f32; 3] {
        lock(&self.state).velocity
    }

    /// Stops the base.
    pub fn stop_move(&self) {
        let mut state = lock(&self.state);
        state.velocity = [0.0; 3];
        state.move_target = None;
    }

    /// Starts a move to a pose relative to the current one, at half the maximum velocities.
    /// Returns the duration of the move: the base stops when it elapses, at the exact target.
    pub fn move_to(&self, x: f32, y: f32, theta: f32) -> Duration {
        let max = self.description.max_velocity;
        let seconds = [x / max[0], y / max[1], theta / max[2]]
            .into_iter()
            .map(|t| t.abs() * 2.0)
            .fold(0.05f32, f32::max);
        let duration = Duration::from_secs_f32(seconds);
        let mut state = lock(&self.state);
        let (sin, cos) = state.pose.theta.sin_cos();
        let target = Pose {
            x: state.pose.x + x * cos - y * sin,
            y: state.pose.y + x * sin + y * cos,
            theta: wrap_angle(state.pose.theta + theta),
        };
        state.velocity = [x / seconds, y / seconds, theta / seconds];
        state.move_target = Some((Instant::now() + duration, target));
        duration
    }

    /// Returns true while a `move_to` is in progress.
    pub fn is_moving(&self) -> bool {
        let state = lock(&self.state);
        state.move_target.is_some() || state.velocity.iter().any(|v| v.abs() > f32::EPSILON)
    }

    /// The pose of the robot in the world frame.
    pub fn pose(&self) -> Pose {
        lock(&self.state).pose
    }

    /// Sets the pose of the robot in the world frame.
    pub fn set_pose(&self, pose: Pose) {
        lock(&self.state).pose = pose;
    }

    /// The position of the torso as `ALMotion.getPosition("Torso", frame, _)` reports it:
    /// `[x, y, z, wx, wy, wz]`.
    pub fn torso_position(&self, frame: i32) -> [f32; 6] {
        let height = self.description.torso_height;
        match frame {
            frame::WORLD => {
                let pose = self.pose();
                [pose.x, pose.y, height, 0.0, 0.0, pose.theta]
            }
            frame::ROBOT => [0.0, 0.0, height, 0.0, 0.0, 0.0],
            _ => [0.0; 6],
        }
    }

    /// Wakes the robot up: stiffness on.
    pub fn wake_up(&self) {
        self.awake.store(true, Ordering::SeqCst);
        let count = self.description.joints.len();
        lock(&self.state).stiffness.fill(1.0);
        let _ = count;
        self.memory.insert("robotIsWakeUp", true);
    }

    /// Rests the robot: stiffness off, base stopped.
    pub fn rest(&self) {
        self.awake.store(false, Ordering::SeqCst);
        {
            let mut state = lock(&self.state);
            state.stiffness.fill(0.0);
            state.velocity = [0.0; 3];
            state.move_target = None;
        }
        self.memory.insert("robotIsWakeUp", false);
    }

    /// Returns true if the robot is awake.
    pub fn is_awake(&self) -> bool {
        self.awake.load(Ordering::SeqCst)
    }

    /// A summary of the state of the joints, as `ALMotion.getSummary` reports it.
    pub fn summary(&self) -> String {
        let state = lock(&self.state);
        let mut summary = String::from(
            "---------------------- Model ---------------------------\n        JointName   Stiffness     Command      Sensor\n",
        );
        for (index, joint) in self.description.joints.iter().enumerate() {
            writeln!(
                summary,
                "{:>17} {:>11.6} {:>11.6} {:>11.6}",
                joint.name, state.stiffness[index], state.targets[index], state.angles[index]
            )
            .ok();
        }
        writeln!(
            summary,
            "---------------------- Pose ----------------------------\nx {:.3} y {:.3} theta {:.3}",
            state.pose.x, state.pose.y, state.pose.theta
        )
        .ok();
        summary
    }

    /// Advances the simulation to the current time, and publishes the sensor keys.
    pub fn tick(&self) {
        let now = Instant::now();
        let mut updates: Vec<(&str, f32)> = Vec::with_capacity(self.keys.len() * 7 + 4);
        let dcm_time =
            i32::try_from(now.duration_since(self.start).as_millis()).unwrap_or(i32::MAX);
        {
            let mut state = lock(&self.state);
            let dt = now.duration_since(state.last_tick).as_secs_f32().min(0.5);
            state.last_tick = now;
            let awake = self.awake.load(Ordering::SeqCst);
            for (index, joint) in self.description.joints.iter().enumerate() {
                let angle = state.angles[index];
                let target = if awake && state.stiffness[index] > 0.0 {
                    state.targets[index]
                } else {
                    angle
                };
                let step = state.fractions[index].max(0.01) * joint.max_velocity * dt;
                let delta = (target - angle).clamp(-step, step);
                let new_angle = angle + delta;
                let velocity = if dt > 0.0 { delta / dt } else { 0.0 };
                state.angles[index] = new_angle;
                state.velocities[index] = velocity;
                let heating = velocity.abs() * 0.02 * dt;
                let cooling = (state.temperatures[index] - REST_TEMPERATURE) * 0.05 * dt;
                state.temperatures[index] += heating - cooling;
                let current = 0.05 + velocity.abs() * 0.3 + state.stiffness[index] * 0.1;
                let torque = velocity * 0.05;
                let keys = &self.keys[index];
                updates.push((&keys.position_sensor, new_angle));
                updates.push((&keys.position_actuator, state.targets[index]));
                updates.push((&keys.current, current));
                updates.push((&keys.temperature, state.temperatures[index]));
                updates.push((&keys.hardness, state.stiffness[index]));
                updates.push((&keys.velocity, velocity));
                updates.push((&keys.torque, torque));
            }
            // Odometry.
            if let Some((deadline, target)) = state.move_target {
                if now >= deadline {
                    state.pose = target;
                    state.velocity = [0.0; 3];
                    state.move_target = None;
                }
            }
            if state.move_target.is_some() || state.velocity.iter().any(|v| *v != 0.0) {
                let [vx, vy, wz] = state.velocity;
                let (sin, cos) = state.pose.theta.sin_cos();
                state.pose.x += (vx * cos - vy * sin) * dt;
                state.pose.y += (vx * sin + vy * cos) * dt;
                state.pose.theta = wrap_angle(state.pose.theta + wz * dt);
            }
            updates.push((
                "Device/SubDeviceList/InertialSensor/GyroscopeZ/Sensor/Value",
                state.velocity[2],
            ));
        }
        self.memory.insert_all_silent(updates);
        self.memory.insert_silent("DCM/Time", dcm_time);
    }

    /// Spawns the task ticking the body at the given period. The task stops when the body is
    /// dropped, or when the returned handle aborts it.
    pub fn spawn_ticker(self: &Arc<Self>, period: Duration) -> tokio::task::AbortHandle {
        let body = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let Some(body) = body.upgrade() else {
                    break;
                };
                body.tick();
            }
        })
        .abort_handle()
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Body")
            .field("model", &self.description.model)
            .field("awake", &self.is_awake())
            .field("pose", &self.pose())
            .finish_non_exhaustive()
    }
}

/// Expands a single value to the size of the targets, or checks that the sizes match.
fn broadcast(count: usize, values: &[f32], what: &str) -> Result<Vec<f32>, String> {
    match values {
        [value] => Ok(vec![*value; count]),
        values if values.len() == count => Ok(values.to_vec()),
        values => Err(format!(
            "the number of {what} ({}) does not match the number of joints ({count})",
            values.len()
        )),
    }
}

fn check_fraction(fraction: f32) -> Result<f32, String> {
    if fraction > 0.0 && fraction <= 1.0 {
        Ok(fraction)
    } else {
        Err(format!(
            "fractionMaxSpeed must be in ]0, 1], got {fraction}"
        ))
    }
}

/// Wraps an angle to `[-pi, pi]`.
fn wrap_angle(angle: f32) -> f32 {
    let two_pi = 2.0 * std::f32::consts::PI;
    let wrapped = (angle + std::f32::consts::PI).rem_euclid(two_pi) - std::f32::consts::PI;
    if wrapped == -std::f32::consts::PI {
        std::f32::consts::PI
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Arc<Body> {
        Body::new(RobotModel::Nao.description(), Memory::new())
    }

    #[test]
    fn joints_move_toward_targets_within_limits() {
        let body = body();
        let head = body.resolve(&["HeadYaw".to_owned()]).unwrap();
        body.set_angles(&head, &[10.0], 1.0).unwrap();
        // The target is clamped to the joint limit.
        assert_eq!(body.angles(&head, false), [2.0857]);
        std::thread::sleep(Duration::from_millis(30));
        body.tick();
        let angle = body.angles(&head, true)[0];
        assert!(angle > 0.0 && angle < 2.0857, "{angle}");
        assert!(body.set_angles(&head, &[0.0], 0.0).is_err());
        assert!(body.set_angles(&head, &[0.0, 1.0], 0.5).is_err());
        let memory_angle = body
            .memory
            .get_f32("Device/SubDeviceList/HeadYaw/Position/Sensor/Value")
            .unwrap();
        assert_eq!(memory_angle, angle);
    }

    #[test]
    fn odometry_integrates_velocity() {
        let body = body();
        body.set_velocity([0.1, 0.0, 0.0]);
        std::thread::sleep(Duration::from_millis(50));
        body.tick();
        let pose = body.pose();
        assert!(pose.x > 0.003 && pose.x < 0.03, "{pose:?}");
        body.stop_move();
        assert_eq!(body.velocity(), [0.0; 3]);
        let position = body.torso_position(frame::WORLD);
        assert_eq!(position[0], pose.x);
        assert_eq!(position[2], 0.3175);
    }

    #[test]
    fn move_to_ends_at_the_target() {
        let body = body();
        let duration = body.move_to(0.02, 0.0, 0.5);
        assert!(duration >= Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(20));
        body.tick();
        assert!(body.is_moving());
        // Force the deadline.
        lock(&body.state).move_target.as_mut().unwrap().0 = Instant::now();
        body.tick();
        let pose = body.pose();
        assert!((pose.x - 0.02).abs() < 1e-5, "{pose:?}");
        assert!((pose.theta - 0.5).abs() < 1e-5, "{pose:?}");
        assert!(!body.is_moving());
    }

    #[test]
    fn rest_freezes_joints() {
        let body = body();
        body.rest();
        assert!(!body.is_awake());
        let head = body.resolve(&["HeadPitch".to_owned()]).unwrap();
        body.set_angles(&head, &[0.3], 1.0).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        body.tick();
        assert_eq!(body.angles(&head, true), [0.0]);
        body.wake_up();
        std::thread::sleep(Duration::from_millis(30));
        body.tick();
        assert!(body.angles(&head, true)[0] > 0.0);
    }
}
