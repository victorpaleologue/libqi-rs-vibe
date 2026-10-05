//! The `getImageRemote` structure and the camera subscriptions.

use crate::common;

use common::Fixture;
use qi::naoqi_sim::{AlValue, RobotModel};
use qi::{value::Value, ObjectExt};

/// Checks the 12-element structure the driver expects, and returns the buffer size and the
/// timestamps.
fn check_image(
    image: &[AlValue],
    width: i32,
    height: i32,
    layers: i32,
    colorspace: i32,
    camera: i32,
) -> (usize, (i32, i32)) {
    assert!(image.len() >= 12, "{} elements", image.len());
    for index in [0, 1, 2, 3, 4, 5, 7] {
        assert!(
            matches!(image[index].value(), Value::Int32(_)),
            "element {index} is {}",
            image[index]
        );
    }
    assert!(
        matches!(image[6].value(), Value::Raw(_)),
        "element 6 is {}",
        image[6]
    );
    for index in [8, 9, 10, 11] {
        assert!(
            matches!(image[index].value(), Value::Float32(_)),
            "element {index} is {}",
            image[index]
        );
    }
    assert_eq!(image[0].as_i32(), Some(width));
    assert_eq!(image[1].as_i32(), Some(height));
    assert_eq!(image[2].as_i32(), Some(layers));
    assert_eq!(image[3].as_i32(), Some(colorspace));
    assert_eq!(image[7].as_i32(), Some(camera));
    let Value::Raw(buffer) = image[6].value() else {
        unreachable!()
    };
    assert_eq!(buffer.len(), (width * height * layers) as usize);
    let fov: Vec<f32> = image[8..12].iter().map(|v| v.as_f32().unwrap()).collect();
    assert!(
        fov[0] > 0.0 && fov[1] > 0.0 && fov[2] < 0.0 && fov[3] < 0.0,
        "{fov:?}"
    );
    (
        buffer.len(),
        (image[4].as_i32().unwrap(), image[5].as_i32().unwrap()),
    )
}

#[tokio::test]
async fn get_image_remote_has_the_driver_structure() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let video = fixture.service("ALVideoDevice").await;
    let handle: String = video
        .call("subscribeCamera", ("front_camera".to_owned(), 0, 1, 11, 10))
        .await
        .unwrap();
    assert_eq!(handle, "front_camera_0");
    let image: Vec<AlValue> = video.call("getImageRemote", handle.clone()).await.unwrap();
    let (_, first) = check_image(&image, 320, 240, 3, 11, 0);
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let image: Vec<AlValue> = video.call("getImageRemote", handle.clone()).await.unwrap();
    let (_, second) = check_image(&image, 320, 240, 3, 11, 0);
    assert!(second > first, "{second:?} > {first:?}");
    let released: bool = video.call("releaseImage", handle.clone()).await.unwrap();
    assert!(released);

    // VGA on the bottom camera, and a second subscription of the same name.
    let vga: String = video
        .call("subscribeCamera", ("front_camera".to_owned(), 1, 2, 11, 30))
        .await
        .unwrap();
    assert_eq!(vga, "front_camera_1");
    let image: Vec<AlValue> = video.call("getImageRemote", vga.clone()).await.unwrap();
    check_image(&image, 640, 480, 3, 11, 1);

    // Unsubscribing invalidates the handle.
    let unsubscribed: bool = video.call("unsubscribe", handle.clone()).await.unwrap();
    assert!(unsubscribed);
    let unsubscribed: bool = video.call("unsubscribe", handle.clone()).await.unwrap();
    assert!(!unsubscribed);
    let err = video
        .call::<Vec<AlValue>, _, _>("getImageRemote", handle)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown subscriber"), "{err}");
    let subscribers: Vec<String> = video.call("getSubscribers", ()).await.unwrap();
    assert_eq!(subscribers, ["front_camera_1"]);

    // Invalid parameters are refused.
    assert!(video
        .call::<String, _, _>("subscribeCamera", ("x".to_owned(), 2, 1, 11, 10))
        .await
        .is_err());
    assert!(video
        .call::<String, _, _>("subscribeCamera", ("x".to_owned(), 0, 99, 11, 10))
        .await
        .is_err());
}

#[tokio::test]
async fn pepper_depth_and_stereo_cameras() {
    let fixture = Fixture::start(RobotModel::Pepper).await;
    let video = fixture.service("ALVideoDevice").await;
    let depth: String = video
        .call("subscribeCamera", ("depth_camera".to_owned(), 2, 9, 17, 10))
        .await
        .unwrap();
    let image: Vec<AlValue> = video.call("getImageRemote", depth).await.unwrap();
    check_image(&image, 640, 360, 2, 17, 2);
    let raw_depth: String = video
        .call("subscribeCamera", ("depth_camera".to_owned(), 2, 1, 23, 10))
        .await
        .unwrap();
    let image: Vec<AlValue> = video.call("getImageRemote", raw_depth).await.unwrap();
    check_image(&image, 320, 240, 2, 23, 2);
    let infrared: String = video
        .call(
            "subscribeCamera",
            ("infrared_camera".to_owned(), 2, 1, 20, 10),
        )
        .await
        .unwrap();
    let image: Vec<AlValue> = video.call("getImageRemote", infrared).await.unwrap();
    check_image(&image, 320, 240, 2, 20, 2);
    let stereo: String = video
        .call(
            "subscribeCamera",
            ("stereo_camera".to_owned(), 3, 15, 11, 10),
        )
        .await
        .unwrap();
    let image: Vec<AlValue> = video.call("getImageRemote", stereo).await.unwrap();
    check_image(&image, 640, 180, 3, 11, 3);
    let has_depth: bool = video.call("hasDepthCamera", ()).await.unwrap();
    assert!(has_depth);
}

#[tokio::test]
async fn legacy_subscribe_uses_the_active_camera() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let video = fixture.service("ALVideoDevice").await;
    let set: bool = video.call("setActiveCamera", 1).await.unwrap();
    assert!(set);
    let active: i32 = video.call("getActiveCamera", ()).await.unwrap();
    assert_eq!(active, 1);
    let handle: String = video
        .call("subscribe", ("legacy".to_owned(), 0, 0, 5))
        .await
        .unwrap();
    let image: Vec<AlValue> = video.call("getImageRemote", handle.clone()).await.unwrap();
    check_image(&image, 160, 120, 1, 0, 1);
    let changed: bool = video
        .call("setResolution", (handle.clone(), 1))
        .await
        .unwrap();
    assert!(changed);
    let resolution: i32 = video.call("getResolution", handle.clone()).await.unwrap();
    assert_eq!(resolution, 1);
    let changed: bool = video
        .call("setColorSpace", (handle.clone(), 13))
        .await
        .unwrap();
    assert!(changed);
    let image: Vec<AlValue> = video.call("getImageRemote", handle).await.unwrap();
    check_image(&image, 320, 240, 3, 13, 1);
}
