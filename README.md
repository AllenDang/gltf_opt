# gltf_opt

A Rust library for optimizing GLB (GL Transmission Format) files by resizing textures and optionally converting them to KTX2/Basis Universal format.

## Description

This library provides functionality to optimize GLB files by:

- Resizing textures to reduce file size
- Converting textures to KTX2 format with Basis Universal compression for better performance
- Optionally removing normal textures to further reduce file size
- Repositioning model pivot point to bottom center

## Features

- Resize textures to a specified dimension
- Convert textures to JPEG, PNG, or KTX2/Basis Universal format
- Remove normal textures to reduce file size
- Center pivot point to bottom center of the model (modifies vertex positions directly)
- Preserve GLB structure and other non-texture data

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
gltf_opt = { git = "https://github.com/AllenDang/gltf_opt" }
```

## Usage

```rust
use gltf_opt::prelude::*;

// Assuming you have a GLB file loaded into a reader
let mut reader = /* your GLB file reader */;

// Optimize the GLB file:
// - new_texture_size: Target size for textures (e.g., 512 for 512x512)
// - remove_normal_texture: Whether to remove normal textures
// - convert_to_ktx2: Whether to convert textures to KTX2/Basis Universal format
// - center_pivot: Whether to move pivot point to bottom center
let optimized_glb = optimize(&mut reader, 512, false, true, false)?;

// Save the optimized GLB to a file
std::fs::write("optimized.glb", optimized_glb)?;
```

### Parameters

- `new_texture_size`: The target size for resizing textures (textures will be resized to new_texture_size x new_texture_size)
- `remove_normal_texture`: If true, normal textures will be removed from the model
- `convert_to_ktx2`: If true, textures will be converted to KTX2 format with Basis Universal compression
- `center_pivot`: If true, the model's pivot point will be moved to the bottom center by modifying vertex positions

## Limitations

- **Skinned meshes**: The `center_pivot` feature does not currently support models with skeleton/skin bindings. Using it on skinned meshes may result in incorrect deformation.

## Dependencies

- [fast_image_resize](https://crates.io/crates/fast_image_resize): For fast image resizing
- [gltf](https://crates.io/crates/gltf): For parsing GLTF/GLB files
- [image](https://crates.io/crates/image): For image loading and encoding
- [ktx2-rw](https://github.com/AllenDang/ktx2-rw): For KTX2 texture handling
- [num_cpus](https://crates.io/crates/num_cpus): For detecting CPU count for parallel processing

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

