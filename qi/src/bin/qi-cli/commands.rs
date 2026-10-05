//! The subcommands of the tool.

use crate::{
    json,
    meta::{self, MemberFilter},
    output::{self, MetaResult, ServiceReport},
};
use anyhow::{anyhow, bail, Context as _, Result};
use futures::{future, StreamExt};
use qi::{
    object::{ActionId, ActionNameOrId},
    service::Info,
    service_directory::{self, ServiceDirectory},
    value::{IntoValue, Type, Value},
    AnyObject, Node, Object,
};
use serde_json::Value as Json;
use std::time::{Duration, SystemTime};
use tokio::time::timeout;

/// The node connected to the space, and the global options.
pub(crate) struct Context {
    pub(crate) node: Node<service_directory::Client>,
    /// Print JSON rather than human-readable renderings.
    pub(crate) json: bool,
    /// The time given to connections to services.
    pub(crate) timeout: Duration,
}

impl Context {
    async fn service(&self, name: &str) -> Result<AnyObject> {
        timeout(self.timeout, self.node.service(name))
            .await
            .map_err(|_elapsed| anyhow!("timed out after {}", seconds(self.timeout)))
            .and_then(|result| result.map_err(anyhow::Error::from))
            .with_context(|| format!("cannot reach service \"{name}\""))
    }
}

pub(crate) fn seconds(duration: Duration) -> String {
    format!("{}s", duration.as_secs_f64())
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InfoOptions {
    /// Only list the identifiers and names of the services.
    pub(crate) list: bool,
    pub(crate) filter: MemberFilter,
}

/// Lists the services of the space, or describes the given ones.
pub(crate) async fn info(ctx: &Context, names: &[String], options: InfoOptions) -> Result<()> {
    let mut services = ctx
        .node
        .service_directory()
        .services()
        .await
        .context("cannot list the services of the space")?;
    services.sort_by_key(Info::id);
    let mut missing = Vec::new();
    let selected: Vec<Info> = if names.is_empty() {
        services
    } else {
        names
            .iter()
            .filter_map(|name| {
                let found = services.iter().find(|info| info.name() == name).cloned();
                if found.is_none() {
                    missing.push(format!("\"{name}\""));
                }
                found
            })
            .collect()
    };

    // The meta objects require a connection to each service: get them concurrently.
    let objects: Vec<Option<Result<AnyObject>>> = if options.list {
        selected.iter().map(|_| None).collect()
    } else {
        future::join_all(selected.iter().map(|info| ctx.service(info.name())))
            .await
            .into_iter()
            .map(Some)
            .collect()
    };
    let reports = selected.iter().zip(&objects).map(|(info, object)| {
        let meta: MetaResult<'_> = object.as_ref().map(|result| {
            result
                .as_ref()
                .map(|object| object.meta())
                .map_err(|error| format!("{error:#}"))
        });
        (info, meta)
    });

    if ctx.json {
        let services: Vec<Json> = reports
            .map(|(info, meta)| output::service_json(info, &meta, options.filter))
            .collect();
        println!("{}", output::pretty_json(&Json::Array(services)));
    } else {
        for (info, meta) in reports {
            print!(
                "{}",
                ServiceReport {
                    info,
                    meta,
                    filter: options.filter,
                }
            );
        }
    }

    if !missing.is_empty() {
        bail!("no service named {} in the space", missing.join(", "));
    }
    Ok(())
}

/// A member that may be called or posted to, with the types of its parameters.
#[derive(Debug)]
struct Callable {
    uid: ActionId,
    signature: String,
    parameters: Vec<Option<Type>>,
    return_type: Option<Type>,
}

impl Callable {
    fn method(method: &qi::object::MetaMethod) -> Self {
        Self {
            uid: method.uid,
            signature: meta::method_signature(method),
            parameters: meta::tuple_types(&method.parameters_signature),
            return_type: method.return_signature.as_type().cloned(),
        }
    }

    fn signal(signal: &qi::object::MetaSignal) -> Self {
        Self {
            uid: signal.uid,
            signature: meta::signal_signature(signal),
            parameters: meta::tuple_types(&signal.signature),
            return_type: Some(Type::Unit),
        }
    }
}

/// Converts the arguments for one of the callables, which are overloads of the same member: the
/// first one whose parameters accept the arguments is selected.
fn convert_args<'c>(
    callables: &'c [Callable],
    args: &[String],
) -> Result<(&'c Callable, Value<'static>)> {
    let same_arity: Vec<_> = callables
        .iter()
        .filter(|callable| callable.parameters.len() == args.len())
        .collect();
    if same_arity.is_empty() {
        let signatures: Vec<_> = callables
            .iter()
            .map(|callable| {
                format!(
                    "{} takes {} argument(s)",
                    callable.signature,
                    callable.parameters.len()
                )
            })
            .collect();
        bail!(
            "{} argument(s) given, but {}",
            args.len(),
            signatures.join(", ")
        );
    }
    let mut failures = Vec::new();
    for callable in same_arity {
        let values: Result<Vec<_>> = callable
            .parameters
            .iter()
            .zip(args)
            .enumerate()
            .map(|(index, (ty, arg))| {
                json::parse_arg(arg, ty.as_ref()).with_context(|| format!("argument {}", index + 1))
            })
            .collect();
        match values {
            Ok(values) => return Ok((callable, Value::Tuple(values))),
            Err(error) => failures.push(format!("{}: {error:#}", callable.signature)),
        }
    }
    bail!("cannot convert the arguments: {}", failures.join("; "))
}

/// Calls a method and prints its result.
pub(crate) async fn call(ctx: &Context, target: &str, args: &[String]) -> Result<()> {
    let (service, method) = meta::split_target(target)?;
    let object = ctx.service(service).await?;
    let callables: Vec<_> = meta::methods_named(object.meta(), method)
        .into_iter()
        .map(Callable::method)
        .collect();
    if callables.is_empty() {
        bail!("service \"{service}\" has no method \"{method}\" (see `qi-cli info {service}`)");
    }
    let (callable, args) = convert_args(&callables, args)?;
    let result = object
        .meta_call(ActionNameOrId::Id(callable.uid), args)
        .await
        .with_context(|| format!("the call to {target} failed"))?;
    if let Some(text) = output::render(&result, callable.return_type.as_ref(), ctx.json) {
        println!("{text}");
    }
    Ok(())
}

/// Calls a method or emits a signal without waiting for a result.
pub(crate) async fn post(ctx: &Context, target: &str, args: &[String]) -> Result<()> {
    let (service, member) = meta::split_target(target)?;
    let object = ctx.service(service).await?;
    let mut callables: Vec<_> = meta::methods_named(object.meta(), member)
        .into_iter()
        .map(Callable::method)
        .collect();
    if let Some(signal) = meta::signal_named(object.meta(), member) {
        callables.push(Callable::signal(signal));
    }
    if callables.is_empty() {
        bail!(
            "service \"{service}\" has no method or signal \"{member}\" (see `qi-cli info {service}`)"
        );
    }
    let (callable, args) = convert_args(&callables, args)?;
    object
        .meta_post(ActionNameOrId::Id(callable.uid), args)
        .await;
    flush_posts(&object).await;
    Ok(())
}

/// Post requests are queued and written to the network by a background task: a round-trip on the
/// same session guarantees that the queued posts were written before the process exits.
async fn flush_posts(object: &AnyObject) {
    /// The `metaObject` special member, that every object bound to the messaging layer implements.
    const META_OBJECT: ActionNameOrId = ActionNameOrId::Id(ActionId(2));
    if let Some(client) = object.as_client() {
        let args = Value::Tuple(vec![u32::from(client.id()).into_value()]);
        if object.meta_call(META_OBJECT, args).await.is_ok() {
            return;
        }
    }
    // The round-trip is not possible: leave some time to the background task.
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// Subscribes to a signal, or to the changes of a property, and prints every value received.
pub(crate) async fn watch(ctx: &Context, target: &str, time: bool) -> Result<()> {
    let (service, member) = meta::split_target(target)?;
    let object = ctx.service(service).await?;
    let meta = object.meta();
    let (uid, types) = if let Some(signal) = meta::signal_named(meta, member) {
        (signal.uid, meta::tuple_types(&signal.signature))
    } else if let Some(property) = meta::property_named(meta, member) {
        (property.uid, vec![property.signature.as_type().cloned()])
    } else {
        bail!(
            "service \"{service}\" has no signal or property \"{member}\" (see `qi-cli info {service}`)"
        );
    };
    let mut events = object
        .meta_subscribe(ActionNameOrId::Id(uid))
        .await
        .with_context(|| format!("cannot subscribe to {target}"))?;

    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    loop {
        tokio::select! {
            result = &mut interrupted => {
                result.context("cannot listen to the interruption signal")?;
                return Ok(());
            }
            event = events.next() => match event {
                Some(params) => println!("{}", render_event(&params, &types, time.then(SystemTime::now), ctx.json)),
                None => bail!("the subscription to {target} ended: the service is gone"),
            },
        }
    }
}

/// Renders the parameters of an event: a single parameter is rendered as the value itself.
fn render_event(
    params: &Value<'_>,
    types: &[Option<Type>],
    time: Option<SystemTime>,
    as_json: bool,
) -> String {
    let (value, ty): (&Value<'_>, Option<Type>) = match params {
        Value::Tuple(items) => match (items.as_slice(), types) {
            ([item], [ty]) => (item, ty.clone()),
            ([], _) => (&Value::Unit, Some(Type::Unit)),
            (items, types) if items.len() == types.len() => {
                (params, Some(Type::tuple_of(types.iter().cloned())))
            }
            _ => (params, None),
        },
        other => (other, None),
    };
    match (time, as_json) {
        (Some(time), true) => serde_json::json!({
            "time": output::utc_timestamp(time),
            "value": json::from_value(value, ty.as_ref()),
        })
        .to_string(),
        (Some(time), false) => format!(
            "[{}] {}",
            output::utc_timestamp(time),
            output::render_line(value, ty.as_ref(), false)
        ),
        (None, as_json) => output::render_line(value, ty.as_ref(), as_json),
    }
}

/// Prints the value of a property.
pub(crate) async fn get(ctx: &Context, target: &str) -> Result<()> {
    let (service, property) = meta::split_target(target)?;
    let object = ctx.service(service).await?;
    let Some(property) = meta::property_named(object.meta(), property) else {
        bail!("service \"{service}\" has no property \"{property}\" (see `qi-cli info {service}`)");
    };
    let value = object
        .meta_property(ActionNameOrId::Id(property.uid))
        .await
        .with_context(|| format!("cannot get the property {target}"))?;
    if let Some(text) = output::render(&value, property.signature.as_type(), ctx.json) {
        println!("{text}");
    }
    Ok(())
}

/// Sets the value of a property.
pub(crate) async fn set(ctx: &Context, target: &str, value: &str) -> Result<()> {
    let (service, property) = meta::split_target(target)?;
    let object = ctx.service(service).await?;
    let Some(property) = meta::property_named(object.meta(), property) else {
        bail!("service \"{service}\" has no property \"{property}\" (see `qi-cli info {service}`)");
    };
    let ty = property.signature.as_type();
    let value = json::parse_arg(value, ty).with_context(|| {
        format!(
            "invalid value for the property {target} of type {}",
            meta::pretty_type(ty)
        )
    })?;
    object
        .meta_set_property(ActionNameOrId::Id(property.uid), value)
        .await
        .with_context(|| format!("cannot set the property {target}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callable(signature: &str, parameters: Vec<Option<Type>>) -> Callable {
        Callable {
            uid: ActionId(100),
            signature: signature.to_owned(),
            parameters,
            return_type: None,
        }
    }

    fn args(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn arguments_select_the_overload() {
        let callables = [
            callable(
                "add::(Int32,Int32)->Int32",
                vec![Some(Type::Int32), Some(Type::Int32)],
            ),
            callable(
                "add::(String,String)->String",
                vec![Some(Type::String), Some(Type::String)],
            ),
            callable("add::(Double)->Double", vec![Some(Type::Float64)]),
        ];
        let (selected, value) = convert_args(&callables, &args(&["1", "2"])).unwrap();
        assert_eq!(selected.signature, "add::(Int32,Int32)->Int32");
        assert_eq!(value, Value::Tuple(vec![Value::Int32(1), Value::Int32(2)]));
        let (selected, value) = convert_args(&callables, &args(&["1", "b"])).unwrap();
        assert_eq!(selected.signature, "add::(String,String)->String");
        assert_eq!(
            value,
            Value::Tuple(vec![
                Value::String("1".to_owned().into()),
                Value::String("b".to_owned().into())
            ])
        );
        let (selected, _) = convert_args(&callables, &args(&["1.5"])).unwrap();
        assert_eq!(selected.signature, "add::(Double)->Double");

        let error = convert_args(&callables, &args(&[]))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "0 argument(s) given, but add::(Int32,Int32)->Int32 takes 2 argument(s), \
             add::(String,String)->String takes 2 argument(s), add::(Double)->Double takes 1 argument(s)"
        );
        let error = convert_args(&callables, &args(&["x"]))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "cannot convert the arguments: add::(Double)->Double: argument 1: invalid JSON"
        );
        let error = convert_args(&callables[..1], &args(&["1", "2.5"]))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "cannot convert the arguments: add::(Int32,Int32)->Int32: argument 2: \
             the number 2.5 is not representable as int32"
        );
    }

    #[test]
    fn events_render_their_parameters() {
        let types = [Some(Type::Int32)];
        let params = Value::Tuple(vec![Value::Int32(3)]);
        assert_eq!(render_event(&params, &types, None, false), "3");
        assert_eq!(render_event(&params, &types, None, true), "3");
        let at = SystemTime::UNIX_EPOCH;
        assert_eq!(
            render_event(&params, &types, Some(at), false),
            "[1970-01-01T00:00:00.000Z] 3"
        );
        assert_eq!(
            render_event(&params, &types, Some(at), true),
            "{\"time\":\"1970-01-01T00:00:00.000Z\",\"value\":3}"
        );
        // No parameter.
        assert_eq!(render_event(&Value::Tuple(vec![]), &[], None, false), "()");
        assert_eq!(render_event(&Value::Tuple(vec![]), &[], None, true), "null");
        // Several parameters, with a structure.
        let types = [
            Some(Type::Int32),
            Some(Type::struct_of(
                "Point",
                [("x", Type::Int8), ("y", Type::Int8)],
            )),
        ];
        let params = Value::Tuple(vec![
            Value::Int32(1),
            Value::Tuple(vec![Value::Int8(2), Value::Int8(3)]),
        ]);
        assert_eq!(
            render_event(&params, &types, None, false),
            "[1,{\"x\":2,\"y\":3}]"
        );
        // Unexpected parameters are rendered as they are.
        assert_eq!(render_event(&params, &[None], None, false), "[1,[2,3]]");
        assert_eq!(render_event(&Value::Int32(1), &types, None, false), "1");
    }
}
