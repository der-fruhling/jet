# `jet`
A program that sends files to a server.

## What is `jet`?

Jet is a program that packs and expands archives that contain a short version
of what is to be contained in the expanded directory. It is intended to be
used in situations where there are many big files that can be downloaded from
the internet.

It is originally intended to be used for a Minecraft server, but could
eventually be usable for basically anything.

It is currently in somewhat early stages of development and should not be used
in a production or high-stakes scenario. **Data loss may occur!**

## Installing `jet`

You need `rust` installed on your machine. [Install with rustup.](https://rustup.rs/)

```
# no cargo publication (yet)

# from git
cargo install --git https://github.com/der-fruhling/jet.git

# for cloned repository
cargo install --path .
```

## Using `jet`

```
Usage: jet <COMMAND>

Commands:
    pack
    unpack
    peek
    expand
    help    Print this message or the help of the given subcommand(s)

Options:
    -h, --help     Print help
    -V, --version  Print version
```

### `jet pack`

Packs a jet-packed archive for use with `jet expand`.

This reads from the current directory by default. It requires that there is a
`jetfuel.xml` file in the current directory (or as the file specified by
`--jetfuel-path`).

`--output` is requires and should point to a file with the extension based on
the `--compression` value:
- `none`: `.jpk`
- `zlib`: `.jpz` (default)

The output file is tar-encoded and compressed with the selected compression
algorithm. The `jetfuel.xml` (or whatever it is called if using `-F`) file
will be embedded in the archive as `@jetfuel.xml` and converted into a
CBOR-encoded `@manifest`, which is actually read by `jet expand`.

```
Usage: jet pack [OPTIONS] --output <OUTPUT> [SOURCE]

Arguments:
    [SOURCE]  [default: .]

Options:
    -o, --output <OUTPUT>
    -F, --jetfuel-path <JETFUEL_PATH>
    -c, --compression <COMPRESSION>    [default: zlib] [possible values: none, zlib]
    -h, --help                         Print help
```

### `jet expand`

Expands a jet-packed archive created with `jet pack`.

This is different from `jet unpack` in that `expand` downloads remote files and
generates run scripts if specified by the jet-packed archive's `@manifest`.

Any files not specified by the `@manifest` are not unpacked into the target.

```
Usage: jet expand [OPTIONS] <SOURCE>

Arguments:
    <SOURCE>

Options:
    -o, --output <OUTPUT>            [default: .]
    -c, --compression <COMPRESSION>  [default: zlib] [possible values: none, zlib]
    -h, --help                       Print help
```

### `jet unpack`

Unpacks a jet-packed archive created with `jet pack`.

This command extracts the archive file. It's behavior can be semi-mimicked
using the standard `tar` command. `jet unpack` has the additional effect
of converting the `@manifest` into a `@manifest.json` in the target directory
so it can be read by human eyes.

```
Usage: jet unpack [OPTIONS] --source <SOURCE> --output <OUTPUT>

Options:
    -s, --source <SOURCE>
    -o, --output <OUTPUT>
    -c, --compression <COMPRESSION>  [default: zlib] [possible values: none, zlib]
    -h, --help                       Print help
```

### `jet peek`

Peeks at the `@jetfuel.xml` file in an archive. May be used to determine
what an archive contains.

**Note:** In an actual jet-packed archive, `@jetfuel.xml` is not used.
`@manifest` may differ and execute actions that are not in `@jetfuel.xml`.
**If you suspect you have received a jet-packed archive that is malicious,
use `jet unpack` and examine the `@manifest.json` instead.**

```
Usage: jet.exe peek [OPTIONS] <FILE>

Arguments:
    <FILE>

Options:
    -c, --compression <COMPRESSION>  [default: zlib] [possible values: none, zlib]
    -h, --help                       Print help
```

### `jet cache clear`

Clears all cached downloaded files. Will ask for confirmation.

### `jet cache show`

Shows the directory used to store jet's cache.
