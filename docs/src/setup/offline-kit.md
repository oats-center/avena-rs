# Installing without internet

Some sites have no usable internet during installation. Prepare a kit on
another Fedora x86-64 machine before the visit, so every step of the [setup
guide](edge-node.md) can run from local files.

| Item | Why |
|---|---|
| `avena-rs` source, including `Cargo.lock` | The services and profiles |
| A vendored Cargo cache (`cargo vendor`) or release binaries built on the same Fedora version | The first build otherwise downloads crates |
| RPMs for the packages in step 1, plus the `nats` CLI RPM | `dnf` cannot reach its mirrors |
| The rustup offline installer or a copied `~/.rustup` and `~/.cargo` | Rust toolchain |
| The LabJack LJM installer for Linux | LabJack driver |
| `podman save` archives of every image named in the Quadlet files | The containers otherwise pull images on first start |
| `apt.creds` and `leaf.creds` for this box, carried separately from the rest | Credentials |
| `edge-code` source, its Python wheels and model, if the box has a camera | Camera software |

On site, load the images with `podman load -i <file>` before step 8, and the
container units will start from them.

Keep the credentials out of the source archives, and check the checksums of
installers, RPMs, binaries and image archives after copying.
