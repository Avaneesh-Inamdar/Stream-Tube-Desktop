#[cfg(target_os = "android")]
pub fn open(
    video_id: &str,
    title: &str,
    channel: &str,
    russian: bool,
    short: bool,
) -> Result<(), String> {
    use crate::shared::android::with_application_class;
    use jni::{JValue, jni_sig, jni_str};

    with_application_class(
        "io.github.shiroksh.streamtube.VideoPlayer",
        |env, activity, player| {
            let id = env.new_string(video_id)?;
            let title = env.new_string(title)?;
            let channel = env.new_string(channel)?;
            env.call_static_method(
            player,
            jni_str!("open"),
            jni_sig!("(Landroid/app/Activity;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ZZ)V"),
            &[JValue::Object(activity), JValue::Object(&id), JValue::Object(&title),
              JValue::Object(&channel), JValue::Bool(russian), JValue::Bool(short)],
        )?;
            Ok(())
        },
    )
}

#[cfg(not(target_os = "android"))]
pub fn open(_: &str, _: &str, _: &str, _: bool, _: bool) -> Result<(), String> {
    Err("The embedded player needs Android".to_owned())
}
