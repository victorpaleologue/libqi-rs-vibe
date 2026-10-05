# qi

A Rust implementation of the `qi` framework: the middleware of the NAO and Pepper
robots (`libqi`, also known as qimessaging). The crate speaks the exact wire format and
protocol of `libqi` 4.x, so that Rust programs interoperate with NAOqi robots and with
C++ `libqi` processes in every role: client, service, or service directory.

Highlights:

- **Nodes and spaces**: connect to a robot (or any service directory), or host a space
  yourself, and publish services.
- **Objects, signals and properties**: call methods with Rust types (arguments and
  results are converted to the signatures advertised by the remote object), subscribe
  to signals as streams, read, write and watch properties, pass objects around and be
  called back.
- **Typed interfaces** with `#[qi::object]`: declare an interface as a trait, get a
  typed client and an object adapter for free.
- **Native futures**: every interaction is an `async fn`; dropping the future of a call
  cancels it on the remote side, and services observe cancellation cooperatively.

## Connecting to a robot

```no_run
use qi::ObjectExt;

/// The NAOqi text-to-speech service, as a typed interface.
#[qi::object(case = "camelCase")]
trait TextToSpeech {
    async fn say(&self, text: String) -> qi::Result<()>;
    async fn get_volume(&self) -> qi::Result<f32>;
}

#[tokio::main]
async fn main() -> qi::Result<()> {
    let node = qi::node::init()
        .connect_to_space("tcp://nao.local:9559".parse().expect("valid address"), None)
        .start()
        .await?;

    // Untyped access: any service, any method. Arguments and results are converted to
    // the types the remote object declares, dynamic (`ALValue`) ones included.
    let memory = node.service("ALMemory").await?;
    let charge: f32 = memory
        .call("getData", "Device/SubDeviceList/Battery/Charge/Sensor/Value".to_owned())
        .await?;

    // Typed access through an interface.
    let tts = TextToSpeechClient::new(node.service("ALTextToSpeech").await?)?;
    let volume = tts.get_volume().await?;
    tts.say(format!("Battery at {:.0} percent, volume {volume}", charge * 100.0))
        .await?;
    Ok(())
}
```

Credentials, when the robot requires them, are passed as the second argument of
`connect_to_space` (`auth_user` and `auth_token` keys of a `KeyDynValueMap`). Robots
running NAOqi 2.9 and later expose an authenticated TLS endpoint: use
`tcps://<robot>:9503`.

## Publishing a service

```no_run
use qi::{Property, Signal};

#[qi::object]
trait Counter {
    /// Adds `step` to the counter and returns the new value.
    async fn increment(&self, step: i32) -> qi::Result<i32>;
    #[qi::signal]
    fn changed(&self) -> &Signal<i32>;
    #[qi::property]
    fn value(&self) -> &Property<i32>;
}

struct MyCounter {
    changed: Signal<i32>,
    value: Property<i32>,
}

#[qi::async_trait]
impl Counter for MyCounter {
    async fn increment(&self, step: i32) -> qi::Result<i32> {
        let value = self.value.get().await? + step;
        self.value.set(value).await?;
        self.changed.emit(value);
        Ok(value)
    }

    fn changed(&self) -> &Signal<i32> {
        &self.changed
    }

    fn value(&self) -> &Property<i32> {
        &self.value
    }
}

#[tokio::main]
async fn main() -> qi::Result<()> {
    let counter = MyCounter {
        changed: Signal::new(),
        value: Property::new(0),
    };
    let mut node = qi::node::init();
    node.add_service("Counter", CounterObject::new(counter));
    node.bind("tcp://0.0.0.0:9559".parse().expect("valid address"));
    // Host the service directory of a new space; use `connect_to_space` instead to join an
    // existing one (a robot) and register the service there.
    let _node = node.host_space().start().await?;
    std::future::pending::<()>().await;
    Ok(())
}
```

Services whose interface is only known at runtime are built with
[`ObjectBuilder`](dynamic::ObjectBuilder) from closures, signals and properties.

## Interoperability

The wire format is checked byte for byte against fixtures produced by `libqi` 4.0.5,
and the crate is tested against C++ `libqi` processes in every combination of roles.
See the `interop/` directory of the repository.
