# issw

Tiny macOS CLI for switching keyboard input sources.

## Install

```sh
cargo install --git https://github.com/0xAndoroid/issw
```

After the crates.io release:

```sh
cargo install issw
```

## Usage

```sh
issw list
issw current
issw Dvorak
issw com.apple.keylayout.Dvorak
```

Switching matches exact id/name first, then a unique case-insensitive substring.
