//! `ALVideoDevice`: the cameras, serving synthesized images.

use super::Context;
use crate::naoqi_sim::{alvalue::AlValue, error, lock, now_secs_usecs, robot::RobotModel};
use qi::{dynamic::ObjectBuilder, value::AsRaw, AnyObject};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicI32, Ordering},
        Arc, Mutex,
    },
};

/// The camera identifiers.
pub mod camera {
    /// The top (front) camera.
    pub const TOP: i32 = 0;
    /// The bottom camera.
    pub const BOTTOM: i32 = 1;
    /// The depth camera (Pepper).
    pub const DEPTH: i32 = 2;
    /// The infrared or stereo camera (Pepper).
    pub const STEREO: i32 = 3;
}

/// The colorspaces of `ALVideoDevice`.
pub mod colorspace {
    /// Luminance only, 1 byte per pixel.
    pub const YUV: i32 = 0;
    /// YUV422, 2 bytes per pixel.
    pub const YUV422: i32 = 9;
    /// RGB, 3 bytes per pixel.
    pub const RGB: i32 = 11;
    /// BGR, 3 bytes per pixel.
    pub const BGR: i32 = 13;
    /// Depth in millimeters, 16 bits per pixel.
    pub const DEPTH: i32 = 17;
    /// Infrared intensity, 16 bits per pixel.
    pub const INFRARED: i32 = 20;
    /// Raw depth, 16 bits per pixel.
    pub const RAW_DEPTH: i32 = 23;
}

/// The size in pixels of a resolution identifier.
pub fn resolution_size(resolution: i32) -> Option<(u32, u32)> {
    Some(match resolution {
        0 => (160, 120),
        1 => (320, 240),
        2 => (640, 480),
        3 => (1280, 960),
        4 => (2560, 1920),
        7 => (80, 60),
        8 => (40, 30),
        9 => (640, 360),
        10 => (320, 180),
        11 => (160, 90),
        12 => (80, 45),
        13 => (1280, 720),
        14 => (1280, 360),
        15 => (640, 180),
        16 => (320, 90),
        17 => (160, 45),
        18 => (2560, 720),
        _ => return None,
    })
}

/// The number of bytes per pixel of a colorspace identifier.
pub fn bytes_per_pixel(colorspace: i32) -> Option<u32> {
    Some(match colorspace {
        0..=8 => 1,
        9 | 14 | 17 | 20 | 21 | 23 => 2,
        10..=13 | 15 | 16 | 22 => 3,
        18 => 4,
        19 | 24 => 12,
        _ => return None,
    })
}

/// A subscription to a camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    /// The name given at subscription.
    pub name: String,
    /// The camera identifier.
    pub camera: i32,
    /// The resolution identifier.
    pub resolution: i32,
    /// The colorspace identifier.
    pub colorspace: i32,
    /// The frame rate, in frames per second.
    pub fps: i32,
    /// The number of images served so far.
    pub frames: u64,
}

/// The state of the video device. Cheap to clone: clones share the state.
#[derive(Clone)]
pub struct VideoDevice(Arc<Inner>);

struct Inner {
    robot: RobotModel,
    logs: crate::naoqi_sim::log::LogHub,
    subscriptions: Mutex<HashMap<String, Subscription>>,
    counters: Mutex<HashMap<String, u32>>,
    parameters: Mutex<HashMap<(i32, i32), i32>>,
    active_camera: AtomicI32,
}

impl VideoDevice {
    /// Creates the video device of a robot.
    pub fn new(context: &Context) -> Self {
        Self(Arc::new(Inner {
            robot: context.robot,
            logs: context.logs.clone(),
            subscriptions: Mutex::default(),
            counters: Mutex::default(),
            parameters: Mutex::default(),
            active_camera: AtomicI32::new(camera::TOP),
        }))
    }

    /// The cameras of the robot.
    pub fn cameras(&self) -> &'static [i32] {
        self.0.robot.description().cameras
    }

    /// The current subscriptions, by handle.
    pub fn subscriptions(&self) -> HashMap<String, Subscription> {
        lock(&self.0.subscriptions).clone()
    }

    /// Subscribes to a camera and returns the handle of the subscription: the name suffixed
    /// with `_<n>`, like NAOqi does.
    pub fn subscribe(
        &self,
        name: &str,
        camera: i32,
        resolution: i32,
        colorspace: i32,
        fps: i32,
    ) -> qi::Result<String> {
        if !self.cameras().contains(&camera) {
            return Err(error(format!(
                "ALVideoDevice::subscribeCamera\n\tinvalid camera index: {camera}"
            )));
        }
        resolution_size(resolution).ok_or_else(|| {
            error(format!(
                "ALVideoDevice::subscribeCamera\n\tinvalid resolution: {resolution}"
            ))
        })?;
        bytes_per_pixel(colorspace).ok_or_else(|| {
            error(format!(
                "ALVideoDevice::subscribeCamera\n\tinvalid colorspace: {colorspace}"
            ))
        })?;
        let handle = {
            let mut counters = lock(&self.0.counters);
            let counter = counters.entry(name.to_owned()).or_default();
            let handle = format!("{name}_{counter}");
            *counter += 1;
            handle
        };
        lock(&self.0.subscriptions).insert(
            handle.clone(),
            Subscription {
                name: name.to_owned(),
                camera,
                resolution,
                colorspace,
                fps: fps.clamp(1, 30),
                frames: 0,
            },
        );
        self.0.logs.info(
            "ALVideoDevice",
            format!("{handle} subscribed to camera {camera} (resolution {resolution}, colorspace {colorspace}, {fps} fps)"),
        );
        Ok(handle)
    }

    /// Unsubscribes a handle. Returns false if it was not subscribed.
    pub fn unsubscribe(&self, handle: &str) -> bool {
        let removed = lock(&self.0.subscriptions).remove(handle).is_some();
        if removed {
            self.0
                .logs
                .info("ALVideoDevice", format!("{handle} unsubscribed"));
        }
        removed
    }

    /// Renders the next image of a subscription as the 12 elements of `getImageRemote`.
    pub fn image(&self, handle: &str) -> qi::Result<Vec<AlValue>> {
        let subscription = {
            let mut subscriptions = lock(&self.0.subscriptions);
            let subscription = subscriptions.get_mut(handle).ok_or_else(|| {
                error(format!(
                    "ALVideoDevice::getImageRemote\n\tunknown subscriber: {handle}"
                ))
            })?;
            subscription.frames += 1;
            subscription.clone()
        };
        let (width, height) = resolution_size(subscription.resolution).unwrap_or((320, 240));
        let layers = bytes_per_pixel(subscription.colorspace).unwrap_or(3);
        let buffer = render(width, height, subscription.colorspace, subscription.frames);
        let (secs, usecs) = now_secs_usecs();
        let [left, top, right, bottom] = field_of_view(self.0.robot, subscription.camera);
        Ok(vec![
            AlValue::from(i32::try_from(width).unwrap_or(i32::MAX)),
            AlValue::from(i32::try_from(height).unwrap_or(i32::MAX)),
            AlValue::from(i32::try_from(layers).unwrap_or(i32::MAX)),
            AlValue::from(subscription.colorspace),
            AlValue::from(secs),
            AlValue::from(usecs),
            AlValue::new(AsRaw(buffer)),
            AlValue::from(subscription.camera),
            AlValue::from(left),
            AlValue::from(top),
            AlValue::from(right),
            AlValue::from(bottom),
        ])
    }

    fn subscription_field<T>(
        &self,
        handle: &str,
        method: &str,
        field: impl Fn(&Subscription) -> T,
    ) -> qi::Result<T> {
        lock(&self.0.subscriptions)
            .get(handle)
            .map(field)
            .ok_or_else(|| {
                error(format!(
                    "ALVideoDevice::{method}\n\tunknown subscriber: {handle}"
                ))
            })
    }

    fn update_subscription(&self, handle: &str, update: impl Fn(&mut Subscription)) -> bool {
        match lock(&self.0.subscriptions).get_mut(handle) {
            Some(subscription) => {
                update(subscription);
                true
            }
            None => false,
        }
    }

    /// Builds the `ALVideoDevice` service object.
    pub fn object(&self) -> AnyObject {
        let device = self.clone();
        let mut builder = ObjectBuilder::new();
        builder.set_description("ALVideoDevice, formerly called Video Input Module/VIM, is architectured in order to provide every module related to vision, a direct access to raw images from video source.");
        method!(builder, "subscribeCamera", [device], |(
            name,
            camera,
            resolution,
            colorspace,
            fps,
        ): (
            String,
            i32,
            i32,
            i32,
            i32
        )| {
            device.subscribe(&name, camera, resolution, colorspace, fps)
        });
        method!(builder, "subscribe", [device], |(
            name,
            resolution,
            colorspace,
            fps,
        ): (
            String,
            i32,
            i32,
            i32
        )| {
            let camera = device.0.active_camera.load(Ordering::SeqCst);
            device.subscribe(&name, camera, resolution, colorspace, fps)
        });
        method!(builder, "unsubscribe", [device], |handle: String| {
            Ok(device.unsubscribe(&handle))
        });
        method!(
            builder,
            "unsubscribeAllInstances",
            [device],
            |name: String| {
                let handles: Vec<String> = lock(&device.0.subscriptions)
                    .iter()
                    .filter(|(_, subscription)| subscription.name == name)
                    .map(|(handle, _)| handle.clone())
                    .collect();
                let any = !handles.is_empty();
                for handle in handles {
                    device.unsubscribe(&handle);
                }
                Ok(any)
            }
        );
        method!(builder, "getImageRemote", [device], |handle: String| {
            device.image(&handle)
        });
        method!(
            builder,
            "getDirectRawImageRemote",
            [device],
            |handle: String| { device.image(&handle) }
        );
        method!(builder, "getImageLocal", [device], |handle: String| {
            device.image(&handle)
        });
        method!(builder, "releaseImage", [device], |handle: String| {
            Ok(lock(&device.0.subscriptions).contains_key(&handle))
        });
        method!(
            builder,
            "releaseDirectRawImage",
            [device],
            |handle: String| { Ok(lock(&device.0.subscriptions).contains_key(&handle)) }
        );
        method!(builder, "getSubscribers", [device], |(): ()| {
            let mut handles: Vec<String> = lock(&device.0.subscriptions).keys().cloned().collect();
            handles.sort();
            Ok(handles)
        });
        method!(builder, "getActiveCamera", [device], |(): ()| {
            Ok(device.0.active_camera.load(Ordering::SeqCst))
        });
        method!(builder, "setActiveCamera", [device], |camera: i32| {
            let valid = device.cameras().contains(&camera);
            if valid {
                device.0.active_camera.store(camera, Ordering::SeqCst);
            }
            Ok(valid)
        });
        method!(builder, "getCameraIndexes", [device], |(): ()| {
            Ok(device.cameras().to_vec())
        });
        method!(builder, "getCameraName", [], |camera: i32| {
            Ok(match camera {
                camera::TOP => "CameraTop",
                camera::BOTTOM => "CameraBottom",
                camera::DEPTH => "CameraDepth",
                camera::STEREO => "CameraStereo",
                _ => "Unknown",
            }
            .to_owned())
        });
        method!(builder, "isCameraOpen", [device], |camera: i32| {
            Ok(device.cameras().contains(&camera))
        });
        method!(builder, "hasDepthCamera", [device], |(): ()| {
            Ok(device.cameras().contains(&camera::DEPTH))
        });
        method!(builder, "getCameraModel", [device], |camera: i32| {
            Ok(match (device.0.robot, camera) {
                (RobotModel::Pepper, camera::DEPTH) => 2,
                (RobotModel::Pepper, camera::STEREO) => 3,
                (RobotModel::Nao, _) => 1,
                _ => 1,
            })
        });
        method!(builder, "setResolution", [device], |(
            handle,
            resolution,
        ): (
            String,
            i32
        )| {
            if resolution_size(resolution).is_none() {
                return Ok(false);
            }
            Ok(device
                .update_subscription(&handle, |subscription| subscription.resolution = resolution))
        });
        method!(builder, "setColorSpace", [device], |(
            handle,
            colorspace,
        ): (
            String,
            i32
        )| {
            if bytes_per_pixel(colorspace).is_none() {
                return Ok(false);
            }
            Ok(device
                .update_subscription(&handle, |subscription| subscription.colorspace = colorspace))
        });
        method!(builder, "setFrameRate", [device], |(handle, fps): (
            String,
            i32
        )| {
            Ok(device
                .update_subscription(&handle, |subscription| subscription.fps = fps.clamp(1, 30)))
        });
        method!(builder, "getResolution", [device], |handle: String| {
            device.subscription_field(&handle, "getResolution", |subscription| {
                subscription.resolution
            })
        });
        method!(builder, "getColorSpace", [device], |handle: String| {
            device.subscription_field(&handle, "getColorSpace", |subscription| {
                subscription.colorspace
            })
        });
        method!(builder, "getFrameRate", [device], |handle: String| {
            device.subscription_field(&handle, "getFrameRate", |subscription| subscription.fps)
        });
        method!(builder, "setParameter", [device], |(
            camera,
            parameter,
            value,
        ): (
            i32,
            i32,
            i32
        )| {
            let valid = device.cameras().contains(&camera);
            if valid {
                lock(&device.0.parameters).insert((camera, parameter), value);
            }
            Ok(valid)
        });
        method!(
            builder,
            "getParameter",
            [device],
            |(camera, parameter): (i32, i32)| {
                Ok(*lock(&device.0.parameters)
                    .get(&(camera, parameter))
                    .unwrap_or(&0))
            }
        );
        method!(builder, "setCameraParameter", [device], |(
            handle,
            parameter,
            value,
        ): (
            String,
            i32,
            i32
        )| {
            let camera =
                device.subscription_field(&handle, "setCameraParameter", |subscription| {
                    subscription.camera
                })?;
            lock(&device.0.parameters).insert((camera, parameter), value);
            Ok(true)
        });
        method!(builder, "getCameraParameter", [device], |(
            handle,
            parameter,
        ): (
            String,
            i32
        )| {
            let camera =
                device.subscription_field(&handle, "getCameraParameter", |subscription| {
                    subscription.camera
                })?;
            Ok(*lock(&device.0.parameters)
                .get(&(camera, parameter))
                .unwrap_or(&0))
        });
        method!(
            builder,
            "setAllParametersToDefault",
            [device],
            |camera: i32| {
                lock(&device.0.parameters).retain(|(cam, _), _| *cam != camera);
                Ok(())
            }
        );
        AnyObject::new(builder.build())
    }
}

impl std::fmt::Debug for VideoDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoDevice")
            .field("subscriptions", &lock(&self.0.subscriptions).len())
            .finish_non_exhaustive()
    }
}

/// The field of view of a camera as `getImageRemote` reports it: left, top, right and bottom
/// angles in radians.
pub fn field_of_view(robot: RobotModel, camera: i32) -> [f32; 4] {
    let (horizontal, vertical): (f32, f32) = match (robot, camera) {
        (_, camera::DEPTH | camera::STEREO) => (58.0, 45.0),
        (RobotModel::Nao, _) => (60.97, 47.64),
        (RobotModel::Pepper, _) => (55.2, 44.3),
    };
    let half_h = horizontal.to_radians() / 2.0;
    let half_v = vertical.to_radians() / 2.0;
    [half_h, half_v, -half_h, -half_v]
}

/// Renders a synthetic image: a color gradient with a bright square moving with the frame
/// number. Depth images hold millimeters, infrared images intensities.
#[allow(clippy::integer_division)]
pub fn render(width: u32, height: u32, colorspace: i32, frame: u64) -> Vec<u8> {
    let bpp = bytes_per_pixel(colorspace).unwrap_or(3) as usize;
    let (width_usize, height_usize) = (width as usize, height as usize);
    let mut buffer = vec![0u8; width_usize * height_usize * bpp];
    let square = (width_usize / 8).max(4);
    let period = (width_usize - square).max(1) as u64;
    let square_x = (frame * 4 % period) as usize;
    let square_y = (height_usize.saturating_sub(square)) / 2;
    for y in 0..height_usize {
        let g = (y * 255 / height_usize.max(1)) as u8;
        for x in 0..width_usize {
            let r = (x * 255 / width_usize.max(1)) as u8;
            let in_square =
                x >= square_x && x < square_x + square && y >= square_y && y < square_y + square;
            let b = (frame % 256) as u8;
            let offset = (y * width_usize + x) * bpp;
            let pixel = &mut buffer[offset..offset + bpp];
            match bpp {
                1 => pixel[0] = if in_square { 255 } else { luminance(r, g, b) },
                2 => {
                    let value: u16 = match colorspace {
                        colorspace::DEPTH | colorspace::RAW_DEPTH | 21 => {
                            if in_square {
                                600
                            } else {
                                800 + (x * 3000 / width_usize.max(1)) as u16
                            }
                        }
                        colorspace::INFRARED => {
                            if in_square {
                                4095
                            } else {
                                u16::from(luminance(r, g, b)) * 16
                            }
                        }
                        _ => u16::from_le_bytes([luminance(r, g, b), 128]),
                    };
                    pixel.copy_from_slice(&value.to_le_bytes());
                }
                3 => {
                    let (r, g, b) = if in_square {
                        (255, 255, 255)
                    } else {
                        (r, g, b)
                    };
                    if colorspace == colorspace::BGR {
                        pixel.copy_from_slice(&[b, g, r]);
                    } else {
                        pixel.copy_from_slice(&[r, g, b]);
                    }
                }
                4 => {
                    let (r, g, b) = if in_square {
                        (255, 255, 255)
                    } else {
                        (r, g, b)
                    };
                    pixel.copy_from_slice(&[255, r, g, b]);
                }
                _ => {
                    // Point clouds: x, y and z as floats.
                    let z = if in_square {
                        0.6f32
                    } else {
                        0.8 + x as f32 / width as f32 * 3.0
                    };
                    let px = (x as f32 / width as f32 - 0.5) * z;
                    let py = (y as f32 / height as f32 - 0.5) * z;
                    pixel[0..4].copy_from_slice(&px.to_le_bytes());
                    pixel[4..8].copy_from_slice(&py.to_le_bytes());
                    pixel[8..12].copy_from_slice(&z.to_le_bytes());
                }
            }
        }
    }
    buffer
}

#[allow(clippy::integer_division)]
fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_have_the_right_size() {
        assert_eq!(render(320, 240, colorspace::RGB, 0).len(), 320 * 240 * 3);
        assert_eq!(render(640, 360, colorspace::DEPTH, 3).len(), 640 * 360 * 2);
        assert_eq!(render(160, 120, colorspace::YUV, 3).len(), 160 * 120);
        assert_eq!(render(80, 60, 19, 1).len(), 80 * 60 * 12);
        // Frames differ.
        assert_ne!(
            render(320, 240, colorspace::RGB, 0),
            render(320, 240, colorspace::RGB, 1)
        );
    }

    #[test]
    fn fields_of_view_are_symmetric() {
        let [left, top, right, bottom] = field_of_view(RobotModel::Nao, camera::TOP);
        assert!(left > 0.5 && left < 0.55);
        assert_eq!(left, -right);
        assert_eq!(top, -bottom);
    }
}
