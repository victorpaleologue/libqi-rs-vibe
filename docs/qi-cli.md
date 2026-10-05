# qi-cli

Command-line tools for `qi` spaces. The crate builds the `qi-cli` command, modeled on the `qicli`
tool of the reference implementation (`libqi`), that explores the services of a space and interacts
with them from a shell.

```console
$ cargo install --path qi --features cli --bin qi-cli
$ qi-cli --help
```

## Usage

```
qi-cli [--url URL] [--user USER --token TOKEN] [--json] [--log-level LEVEL] [--timeout SECONDS] <COMMAND>
```

The space is designated by the address of its service directory, `tcp://localhost:9559` by default
(`--url`, or `--qi-url` as in `qicli`). Spaces that require authentication are joined with `--user`
and `--token`. Members of services are designated as `SERVICE.MEMBER`.

### Listing and describing services

```console
$ qi-cli info                     # every service, with its members
$ qi-cli info --list              # only the identifiers and names of the services
$ qi-cli info Calculator          # one service
002 [Calculator]
  * Info:
    machine   9a65b56e-c3d3-4485-8924-661b036202b3
    process   3420486
    endpoints tcp://127.0.0.1:41681
              qi:ServiceDirectory
  * Description: A test calculator
  * Methods:
    100 add::(Int32,Int32)->Int32
    101 concat::(String,String)->String
  * Signals:
    105 fired::(Int32)
  * Properties:
    106 value::Int32
    107 text::String
```

Methods are printed as `name::(parameters)->result`, with type names in the style of `qicli`
(`Int32`, `Float`, `Double`, `String`, `List<...>`, `Map<...,...>`, `Optional<...>`, `Value` for
dynamic values, and the names of structures).

Members whose names start with an underscore are hidden unless `--hidden` (`-z`) is given. The
special members of the messaging protocol (identifiers below 100) are hidden unless `--details`
(`-d`) is given, which also prints the raw signatures (`(ii)->i`) and the descriptions of the
members and of their parameters.

### Calling methods

```console
$ qi-cli call Calculator.add 40 2
42
$ qi-cli call Calculator.concat foo '"bar"'
foobar
$ qi-cli call Calculator.echo_list '[1, 2, 3]'
[
  1,
  2,
  3
]
$ qi-cli post Calculator.fire 3               # fire and forget
```

The arguments are JSON documents, converted to the types of the parameters of the method as
advertised by its meta object: `1` becomes an `int32` or a `float32` according to the signature.
When a method is overloaded, the first overload whose parameters accept the arguments is called.

| Type of the parameter | Accepted JSON                                                                 |
|-----------------------|-------------------------------------------------------------------------------|
| integers              | numbers representable in the type (`2.0` is accepted, `2.5` is not)           |
| floats                | numbers, or the strings `"NaN"`, `"inf"` and `"-inf"`                         |
| bool                  | `true`, `false`                                                               |
| string                | a JSON string, or any other text taken as the string itself (`foo` and `"foo"` are the same) |
| raw                   | a base64 string, or an array of bytes                                         |
| optional              | `null`, or the value                                                          |
| list                  | an array                                                                      |
| map                   | an object (the keys are converted to the key type: `{"1": "a"}` for a map of `int32` keys), or an array of `[key, value]` pairs for other key types |
| tuple, structure      | an array; an object with the names of the fields for structures (absent optional fields are `null`) |
| dynamic (`Value`)     | any JSON, its type being inferred: integers become `int32` when they fit (`int64` or `uint64` otherwise), other numbers `float64`, arrays lists, objects maps |

`post` also emits signals: `qi-cli post Calculator.fired 3` triggers the signal on the service.

### Signals and properties

```console
$ qi-cli watch Calculator.fired               # one value per line, until Ctrl-C
1
2
$ qi-cli watch Calculator.value --time        # properties are signals of their changes
[2026-09-29T16:42:03.123Z] 7
$ qi-cli get Calculator.value
7
$ qi-cli set Calculator.value 8
$ qi-cli set Calculator.text hello
```

### Output

Results are printed as JSON (structures with named fields as objects, raw data as base64 strings,
non-finite floats as the strings `"NaN"`, `"inf"` and `"-inf"`), with the following conveniences in
the default human-readable mode:

- strings are printed as they are, without quotes;
- raw data is printed as a hexadecimal dump;
- nothing is printed for `void` results.

`--json` disables these conveniences and makes `info` print a JSON array describing the services,
for scripting:

```console
$ qi-cli --json get Calculator.text
"hello"
$ qi-cli --json info Calculator | jq '.[0].methods[].signature'
"add::(Int32,Int32)->Int32"
"concat::(String,String)->String"
$ qi-cli --json watch Calculator.fired --time
{"time":"2026-09-29T16:42:03.123Z","value":1}
```

Errors are reported on the standard error and make the command exit with a non-zero status
(1 for failures, 2 for usage errors).

### Logs

The logs of the `qi` crates are printed on the standard error, filtered by `--log-level` or the
`RUST_LOG` environment variable (`warn` by default). Both accept a level (`error`, `warn`, `info`,
`debug`, `trace`) or `tracing` filter directives such as `qi=debug`.

## Development

```console
$ cargo test -p libqi-vibe --features cli
$ cargo run -p libqi-vibe --features cli --bin qi-cli -- --url tcp://localhost:9559 info
```

The conversions between JSON and values of the type system live in `src/json.rs`.
