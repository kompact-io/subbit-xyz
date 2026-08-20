# subbit

The Subbit.xyz validator(s).

## Building

For testing, add tracing

```sh
aiken build --trace-filter all --trace-level verbose
```

For prod

```sh
aiken build
```

## Tests & Bench

Are tests are in `./lib/mark` ("test" and "bench" are keywords, hence "mark").

```sh
aiken check
```

## Documentation

See the spec
