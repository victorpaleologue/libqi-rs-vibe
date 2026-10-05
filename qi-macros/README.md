# libqi-macros-vibe

Procedural macros of the `qi` framework. The crate is not used directly: the macros are
re-exported by the [`libqi-vibe`](https://crates.io/crates/libqi-vibe) crate, used as `qi`.

- `#[qi::object]` turns a trait into a `qi` interface: a meta object, an object adapter
  that exposes an implementation to the network, and a typed client for remote objects.
- `#[derive(Valuable)]` and the companion derives (`Reflect`, `ToValue`, `FromValue`,
  `IntoValue`) map Rust structs and enums to `qi` types, tuples, structures and values.

The `qi` crate documentation describes the attributes and shows the macros in use.

## License

BSD 3-Clause, see the [`LICENSE`](https://github.com/victorpaleologue/libqi-rs-vibe/blob/main/LICENSE)
file of the repository.
