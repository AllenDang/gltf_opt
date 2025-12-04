use std::{
    borrow::Cow,
    error::Error,
    io::{Cursor, Read, Seek, Write},
};

use fast_image_resize::IntoImageView;
use gltf::json::{Index, Root, Texture, image::MimeType, mesh::Primitive};
use image::{
    ImageEncoder,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
};
use ktx2_rw::{BasisCompressionParams, Ktx2Texture};

/// Enum to specify the type of texture for appropriate compression settings
#[derive(Debug, Clone, Copy)]
enum TextureType {
    BaseColor,         // sRGB color textures
    Normal,            // Normal maps (need higher quality)
    MetallicRoughness, // Material property textures
}

fn resize_to_jpg<W: Write>(
    img_data: &[u8],
    width: u32,
    height: u32,
    mut buf: W,
) -> Result<(), Box<dyn Error>> {
    let img = image::load_from_memory(img_data)?;

    // Only resize if image dimensions are greater than target dimensions
    if img.width() > width || img.height() > height {
        let mut dst_img = fast_image_resize::images::Image::new(
            width,
            height,
            img.pixel_type().ok_or("failed to create resize image")?,
        );

        let mut resizer = fast_image_resize::Resizer::new();
        resizer.resize(&img, &mut dst_img, None)?;

        JpegEncoder::new(&mut buf).write_image(
            dst_img.buffer(),
            width,
            height,
            img.color().into(),
        )?;
    } else {
        // If image is smaller or equal to target size, just copy it as is
        JpegEncoder::new(&mut buf).write_image(
            img.as_bytes(),
            img.width(),
            img.height(),
            img.color().into(),
        )?;
    }

    Ok(())
}

fn resize_to_png<W: Write>(
    img_data: &[u8],
    width: u32,
    height: u32,
    mut buf: W,
) -> Result<(), Box<dyn Error>> {
    let img = image::load_from_memory(img_data)?;

    // Only resize if image dimensions are greater than target dimensions
    if img.width() > width || img.height() > height {
        let mut dst_img = fast_image_resize::images::Image::new(
            width,
            height,
            img.pixel_type().ok_or("failed to create resize image")?,
        );

        let mut resizer = fast_image_resize::Resizer::new();
        resizer.resize(&img, &mut dst_img, None)?;

        PngEncoder::new(&mut buf).write_image(
            dst_img.buffer(),
            width,
            height,
            img.color().into(),
        )?;
    } else {
        // If image is smaller or equal to target size, just copy it as is
        PngEncoder::new(&mut buf).write_image(
            img.as_bytes(),
            img.width(),
            img.height(),
            img.color().into(),
        )?;
    }

    Ok(())
}

/// Unified function to resize and convert images to KTX2 with Basis Universal compression
/// Uses appropriate compression settings based on texture type
fn resize_to_ktx2<W: Write>(
    img_data: &[u8],
    width: u32,
    height: u32,
    texture_type: TextureType,
    mut buf: W,
) -> Result<(), Box<dyn Error>> {
    let img = image::load_from_memory(img_data)?;

    // Get compression parameters based on texture type
    let (quality_level, endpoint_rdo, selector_rdo) = match texture_type {
        TextureType::Normal => (180, 1.0, 1.0), // Higher quality for normal maps
        TextureType::BaseColor | TextureType::MetallicRoughness => (150, 1.25, 1.25), // Standard quality
    };

    // Only resize if image dimensions are greater than target dimensions
    if img.width() > width || img.height() > height {
        let rgba_img = img.to_rgba8();

        let src_img = fast_image_resize::images::Image::from_vec_u8(
            img.width(),
            img.height(),
            rgba_img.into_raw(),
            fast_image_resize::PixelType::U8x4, // Always RGBA8
        )?;

        let mut dst_img = fast_image_resize::images::Image::new(
            width,
            height,
            fast_image_resize::PixelType::U8x4,
        );

        let mut resizer = fast_image_resize::Resizer::new();
        resizer.resize(&src_img, &mut dst_img, None)?;

        // All texture types now use R8G8B8A8Unorm (linear) format
        let mut ktx2_tex =
            Ktx2Texture::create(width, height, 1, 1, 1, 1, ktx2_rw::VkFormat::R8G8B8A8Unorm)?;
        ktx2_tex.set_image_data(0, 0, 0, dst_img.buffer())?;
        ktx2_tex.set_metadata("Tool", b"glb_opt")?;
        ktx2_tex.set_metadata("Dimensions", format!("{width}x{height}").as_bytes())?;

        let etc1s_params = BasisCompressionParams::builder()
            .uastc(false)
            .thread_count(num_cpus::get() as u32)
            .quality_level(quality_level)
            .endpoint_rdo_threshold(endpoint_rdo)
            .selector_rdo_threshold(selector_rdo)
            .build();
        ktx2_tex.compress_basis(&etc1s_params)?;
        ktx2_tex.set_metadata("CompressionMode", b"ETC1S")?;

        let ktx2_data = ktx2_tex.write_to_memory()?;
        buf.write_all(&ktx2_data)?;
    } else {
        // If image is smaller or equal to target size, use original image data directly
        let rgba_img = img.to_rgba8();
        let img_data = rgba_img.as_raw();

        let mut ktx2_tex = Ktx2Texture::create(
            img.width(),
            img.height(),
            1,
            1,
            1,
            1,
            ktx2_rw::VkFormat::R8G8B8A8Unorm,
        )?;
        ktx2_tex.set_image_data(0, 0, 0, img_data)?;
        ktx2_tex.set_metadata("Tool", b"glb_opt")?;
        ktx2_tex.set_metadata(
            "Dimensions",
            format!("{}x{}", img.width(), img.height()).as_bytes(),
        )?;

        let etc1s_params = BasisCompressionParams::builder()
            .uastc(false)
            .thread_count(num_cpus::get() as u32)
            .quality_level(quality_level)
            .endpoint_rdo_threshold(endpoint_rdo)
            .selector_rdo_threshold(selector_rdo)
            .build();
        ktx2_tex.compress_basis(&etc1s_params)?;
        ktx2_tex.set_metadata("CompressionMode", b"ETC1S")?;

        let ktx2_data = ktx2_tex.write_to_memory()?;
        buf.write_all(&ktx2_data)?;
    }

    Ok(())
}

/// Helper function to update image name/URI when converting to KTX2
fn update_image_name_for_ktx2(image_name: &Option<String>) -> Option<String> {
    if let Some(name) = image_name {
        // Replace common image extensions with .ktx2
        let updated_name = name
            .replace(".jpg", ".ktx2")
            .replace(".jpeg", ".ktx2")
            .replace(".png", ".ktx2")
            .replace(".webp", ".ktx2")
            .replace(".JPG", ".ktx2")
            .replace(".JPEG", ".ktx2")
            .replace(".PNG", ".ktx2")
            .replace(".WEBP", ".ktx2");

        // If no extension was found, just append .ktx2
        if updated_name == *name {
            Some(format!("{name}.ktx2"))
        } else {
            Some(updated_name)
        }
    } else {
        None
    }
}

fn add_image(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    img: &gltf::json::Image,
    b: &[u8],
    mime_type: &str,
) -> Index<gltf::json::Image> {
    let offset = n_blob.len();
    let length = b.len();

    n_blob.extend_from_slice(b);

    let view = gltf::json::buffer::View {
        buffer: Index::<gltf::json::buffer::Buffer>::new(0),
        byte_length: length.into(),
        byte_offset: if offset == 0 {
            None
        } else {
            Some(offset.into())
        },
        byte_stride: None,
        name: None,
        target: None,
        extensions: None,
        extras: Default::default(),
    };

    let view_idx = n_json.push(view);

    let mut n_img = img.clone();

    n_img.buffer_view = Some(view_idx);
    n_img.mime_type = Some(MimeType(mime_type.to_string()));

    // Update name and URI when converting to KTX2
    if mime_type == "image/ktx2" {
        // Update existing name/URI if they exist
        n_img.name = update_image_name_for_ktx2(&img.name);
        n_img.uri = update_image_name_for_ktx2(&img.uri);

        // If no name was set, generate a default KTX2 name
        if n_img.name.is_none() {
            n_img.name = Some(format!("texture_{}.ktx2", n_json.images.len()));
        }
    }

    n_json.push(n_img)
}

fn add_accessor(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    idx: Index<gltf::json::Accessor>,
) -> Option<Index<gltf::json::Accessor>> {
    add_accessor_with_offset(n_blob, n_json, o_blob, o_json, idx, None)
}

/// Add accessor with optional position offset for POSITION attributes
fn add_accessor_with_offset(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    idx: Index<gltf::json::Accessor>,
    position_offset: Option<[f32; 3]>,
) -> Option<Index<gltf::json::Accessor>> {
    if let Some(acc) = o_json.accessors.get(idx.value())
        && let Some(idx_view) = acc.buffer_view
        && let Some(view) = o_json.buffer_views.get(idx_view.value())
    {
        let offset = match view.byte_offset {
            Some(o) => o.0 as usize,
            None => 0,
        };
        let length = view.byte_length.0 as usize;

        if let Some(data) = &o_blob.get(offset..(offset + length)) {
            let n_offset = n_blob.len();

            // If we have a position offset and this is a VEC3 accessor, apply the offset
            if let Some(pos_offset) = position_offset {
                let acc_offset = acc.byte_offset.map(|o| o.0 as usize).unwrap_or(0);
                let stride = view.byte_stride.map(|s| s.0 as usize).unwrap_or(12); // 3 * f32 = 12 bytes
                let count = acc.count.0 as usize;

                let mut modified_data = data.to_vec();

                for i in 0..count {
                    let start = acc_offset + i * stride;
                    if start + 12 <= modified_data.len() {
                        // Read current position
                        let x = f32::from_le_bytes([
                            modified_data[start],
                            modified_data[start + 1],
                            modified_data[start + 2],
                            modified_data[start + 3],
                        ]);
                        let y = f32::from_le_bytes([
                            modified_data[start + 4],
                            modified_data[start + 5],
                            modified_data[start + 6],
                            modified_data[start + 7],
                        ]);
                        let z = f32::from_le_bytes([
                            modified_data[start + 8],
                            modified_data[start + 9],
                            modified_data[start + 10],
                            modified_data[start + 11],
                        ]);

                        // Apply offset
                        let new_x = x + pos_offset[0];
                        let new_y = y + pos_offset[1];
                        let new_z = z + pos_offset[2];

                        // Write back
                        modified_data[start..start + 4].copy_from_slice(&new_x.to_le_bytes());
                        modified_data[start + 4..start + 8].copy_from_slice(&new_y.to_le_bytes());
                        modified_data[start + 8..start + 12].copy_from_slice(&new_z.to_le_bytes());
                    }
                }

                n_blob.extend_from_slice(&modified_data);

                // Update accessor min/max values
                let mut n_acc = acc.clone();
                if let Some(ref mut min_val) = n_acc.min {
                    if let Some(min_arr) = min_val.as_array_mut() {
                        if min_arr.len() >= 3 {
                            if let (Some(x), Some(y), Some(z)) = (
                                min_arr[0].as_f64(),
                                min_arr[1].as_f64(),
                                min_arr[2].as_f64(),
                            ) {
                                min_arr[0] = (x as f32 + pos_offset[0]).into();
                                min_arr[1] = (y as f32 + pos_offset[1]).into();
                                min_arr[2] = (z as f32 + pos_offset[2]).into();
                            }
                        }
                    }
                }
                if let Some(ref mut max_val) = n_acc.max {
                    if let Some(max_arr) = max_val.as_array_mut() {
                        if max_arr.len() >= 3 {
                            if let (Some(x), Some(y), Some(z)) = (
                                max_arr[0].as_f64(),
                                max_arr[1].as_f64(),
                                max_arr[2].as_f64(),
                            ) {
                                max_arr[0] = (x as f32 + pos_offset[0]).into();
                                max_arr[1] = (y as f32 + pos_offset[1]).into();
                                max_arr[2] = (z as f32 + pos_offset[2]).into();
                            }
                        }
                    }
                }

                // create buffer_view
                let mut n_view = view.clone();
                n_view.byte_offset = Some(n_offset.into());
                n_view.byte_length = modified_data.len().into();

                let n_view_idx = n_json.push(n_view);

                n_acc.buffer_view = Some(n_view_idx);
                let idx_acc = n_json.push(n_acc);

                return Some(idx_acc);
            } else {
                let length = data.len();
                n_blob.extend_from_slice(data);

                // create buffer_view
                let mut n_view = view.clone();
                n_view.byte_offset = Some(n_offset.into());
                n_view.byte_length = length.into();

                let n_view_idx = n_json.push(n_view);

                // create accessor
                let mut n_acc = acc.clone();
                n_acc.buffer_view = Some(n_view_idx);

                let idx_acc = n_json.push(n_acc);

                return Some(idx_acc);
            }
        }
    }

    None
}

fn get_image_data<'a>(
    o_blob: &'a [u8],
    o_json: &gltf::json::Root,
    texture_idx: Index<Texture>,
) -> Option<&'a [u8]> {
    // Validate texture exists
    let tex = o_json.textures.get(texture_idx.value())?;

    // Validate image exists
    let img = o_json.images.get(tex.source.value())?;

    // Check if image has buffer_view (embedded data)
    let idx_view = img.buffer_view?;

    // Validate buffer_view exists
    let view = o_json.buffer_views.get(idx_view.value())?;

    // Calculate offset and length
    let offset = match view.byte_offset {
        Some(o) => o.0 as usize,
        None => 0,
    };
    let length = view.byte_length.0 as usize;

    // Validate the range is within the blob bounds
    if offset.saturating_add(length) > o_blob.len() {
        return None;
    }

    // Return the image data slice
    o_blob.get(offset..(offset + length))
}

fn add_texture(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    info: &gltf::json::texture::Info,
    n_tex_size: u32,
    convert_to_ktx2: bool,
) -> Result<gltf::json::texture::Info, Box<dyn Error>> {
    let bct_image_data = get_image_data(o_blob, o_json, info.index).ok_or_else(|| {
        format!(
            "Failed to get base color texture image data (texture index: {})",
            info.index.value()
        )
    })?;

    let mut new_bct_data: Vec<u8> = Vec::new();
    let mut writer = Cursor::new(&mut new_bct_data);

    let mime_type = if convert_to_ktx2 {
        "image/ktx2"
    } else {
        "image/jpeg"
    };

    if convert_to_ktx2 {
        resize_to_ktx2(
            bct_image_data,
            n_tex_size,
            n_tex_size,
            TextureType::BaseColor,
            &mut writer,
        )?;
    } else {
        resize_to_jpg(bct_image_data, n_tex_size, n_tex_size, &mut writer)?;
    }

    // Get texture with proper error handling
    let original_texture = o_json
        .textures
        .get(info.index.value())
        .ok_or("Failed to get original texture")?;

    // Get image with proper error handling
    let new_image = o_json
        .images
        .get(original_texture.source.value())
        .ok_or("Failed to get original image")?
        .clone();

    let idx_img = add_image(
        n_blob,
        n_json,
        &new_image,
        &new_bct_data.to_vec(),
        mime_type,
    );

    // Clone texture after validation
    let mut new_tex = original_texture.clone();
    new_tex.source = idx_img;

    let idx_tex = n_json.push(new_tex);

    let mut new_info = info.clone();
    new_info.index = idx_tex;

    Ok(new_info)
}

fn add_normal_texture(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    normal: &gltf::json::material::NormalTexture,
    n_tex_size: u32,
    convert_to_ktx2: bool,
) -> Result<gltf::json::material::NormalTexture, Box<dyn Error>> {
    let bct_image_data = get_image_data(o_blob, o_json, normal.index).ok_or_else(|| {
        format!(
            "Failed to get normal texture image data (texture index: {})",
            normal.index.value()
        )
    })?;

    let mut new_bct_data: Vec<u8> = Vec::new();
    let mut writer = Cursor::new(&mut new_bct_data);

    let mime_type = if convert_to_ktx2 {
        "image/ktx2"
    } else {
        "image/png"
    };

    if convert_to_ktx2 {
        resize_to_ktx2(
            bct_image_data,
            n_tex_size,
            n_tex_size,
            TextureType::Normal,
            &mut writer,
        )?;
    } else {
        resize_to_png(bct_image_data, n_tex_size, n_tex_size, &mut writer)?;
    }

    // Get texture with proper error handling
    let original_texture = o_json
        .textures
        .get(normal.index.value())
        .ok_or("Failed to get original normal texture")?;

    // Get image with proper error handling
    let new_image = o_json
        .images
        .get(original_texture.source.value())
        .ok_or("Failed to get original normal texture image")?
        .clone();

    let idx_img = add_image(
        n_blob,
        n_json,
        &new_image,
        &new_bct_data.to_vec(),
        mime_type,
    );

    // Clone texture after validation
    let mut new_tex = original_texture.clone();
    new_tex.source = idx_img;

    let idx_tex = n_json.push(new_tex);

    let mut new_normal = normal.clone();
    new_normal.index = idx_tex;

    Ok(new_normal)
}

fn add_metallic_roughness_texture(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    info: &gltf::json::texture::Info,
    n_tex_size: u32,
    convert_to_ktx2: bool,
) -> Result<gltf::json::texture::Info, Box<dyn Error>> {
    let bct_image_data = get_image_data(o_blob, o_json, info.index).ok_or_else(|| {
        format!(
            "Failed to get metallic/roughness texture image data (texture index: {})",
            info.index.value()
        )
    })?;

    let mut new_bct_data: Vec<u8> = Vec::new();
    let mut writer = Cursor::new(&mut new_bct_data);

    let mime_type = if convert_to_ktx2 {
        "image/ktx2"
    } else {
        "image/jpeg"
    };

    if convert_to_ktx2 {
        resize_to_ktx2(
            bct_image_data,
            n_tex_size,
            n_tex_size,
            TextureType::MetallicRoughness,
            &mut writer,
        )?;
    } else {
        resize_to_jpg(bct_image_data, n_tex_size, n_tex_size, &mut writer)?;
    }

    // Get texture with proper error handling
    let original_texture = o_json
        .textures
        .get(info.index.value())
        .ok_or("Failed to get original metallic/roughness texture")?;

    // Get image with proper error handling
    let new_image = o_json
        .images
        .get(original_texture.source.value())
        .ok_or("Failed to get original metallic/roughness texture image")?
        .clone();

    let idx_img = add_image(
        n_blob,
        n_json,
        &new_image,
        &new_bct_data.to_vec(),
        mime_type,
    );

    // Clone texture after validation
    let mut new_tex = original_texture.clone();
    new_tex.source = idx_img;

    let idx_tex = n_json.push(new_tex);

    let mut new_info = info.clone();
    new_info.index = idx_tex;

    Ok(new_info)
}

#[allow(clippy::too_many_arguments)]
fn add_primitive(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    p: &gltf::json::mesh::Primitive,
    n_tex_size: u32,
    remove_normal_texture: bool,
    convert_to_ktx2: bool,
    pivot_offset: Option<[f32; 3]>,
) -> Result<Primitive, Box<dyn Error>> {
    let mut n_p = p.clone();

    // copy indices
    if let Some(indices) = p.indices {
        n_p.indices = add_accessor(n_blob, n_json, o_blob, o_json, indices);
    }

    // copy attributes
    n_p.attributes.clear();
    for (k, v) in &p.attributes {
        // Apply pivot offset only to POSITION attributes
        let offset_to_apply = if matches!(
            k,
            gltf::json::validation::Checked::Valid(gltf::json::mesh::Semantic::Positions)
        ) {
            pivot_offset
        } else {
            None
        };

        if let Some(idx_acc) =
            add_accessor_with_offset(n_blob, n_json, o_blob, o_json, *v, offset_to_apply)
        {
            n_p.attributes.insert(k.clone(), idx_acc);
        }
    }

    // add material
    if let Some(idx_mat) = p.material
        && let Some(mat) = o_json.materials.get(idx_mat.value())
    {
        let mut n_mat = mat.clone();

        // resize base color tex
        if let Some(bct_info) = &mat.pbr_metallic_roughness.base_color_texture {
            match add_texture(
                n_blob,
                n_json,
                o_blob,
                o_json,
                bct_info,
                n_tex_size,
                convert_to_ktx2,
            ) {
                Ok(new_info) => {
                    n_mat.pbr_metallic_roughness.base_color_texture = Some(new_info);
                }
                Err(e) => {
                    return Err(format!("Failed to process base color texture: {e}").into());
                }
            }
        }

        // resize metal/rough tex
        if let Some(mr_info) = &mat.pbr_metallic_roughness.metallic_roughness_texture {
            match add_metallic_roughness_texture(
                n_blob,
                n_json,
                o_blob,
                o_json,
                mr_info,
                n_tex_size / 2,
                convert_to_ktx2,
            ) {
                Ok(new_info) => {
                    n_mat.pbr_metallic_roughness.metallic_roughness_texture = Some(new_info);
                }
                Err(e) => {
                    return Err(format!("Failed to process metallic/roughness texture: {e}").into());
                }
            }
        }

        if remove_normal_texture {
            n_mat.normal_texture = None;
        } else {
            // resize normal map
            if let Some(normal_tex) = &mat.normal_texture {
                match add_normal_texture(
                    n_blob,
                    n_json,
                    o_blob,
                    o_json,
                    normal_tex,
                    n_tex_size,
                    convert_to_ktx2,
                ) {
                    Ok(new_normal) => {
                        n_mat.normal_texture = Some(new_normal);
                    }
                    Err(e) => {
                        return Err(format!("Failed to process normal texture: {e}").into());
                    }
                }
            }
        }

        // update material
        let idx_mat = n_json.push(n_mat);
        n_p.material = Some(idx_mat);
    }

    Ok(n_p)
}

fn pad_to_4bytes(data: &mut Vec<u8>) {
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
}

/// Get raw position data from an accessor as f32 vec3 values
fn get_position_data(
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    accessor_idx: Index<gltf::json::Accessor>,
) -> Option<Vec<[f32; 3]>> {
    let acc = o_json.accessors.get(accessor_idx.value())?;
    let idx_view = acc.buffer_view?;
    let view = o_json.buffer_views.get(idx_view.value())?;

    let offset = view.byte_offset.map(|o| o.0 as usize).unwrap_or(0);
    let acc_offset = acc.byte_offset.map(|o| o.0 as usize).unwrap_or(0);
    let stride = view.byte_stride.map(|s| s.0).unwrap_or(12); // 3 * f32
    let count = acc.count.0 as usize;

    let data = o_blob.get(offset..)?;

    let mut positions = Vec::with_capacity(count);
    for i in 0..count {
        let start = acc_offset + i * stride;
        if start + 12 > data.len() {
            return None;
        }
        let x = f32::from_le_bytes([
            data[start],
            data[start + 1],
            data[start + 2],
            data[start + 3],
        ]);
        let y = f32::from_le_bytes([
            data[start + 4],
            data[start + 5],
            data[start + 6],
            data[start + 7],
        ]);
        let z = f32::from_le_bytes([
            data[start + 8],
            data[start + 9],
            data[start + 10],
            data[start + 11],
        ]);
        positions.push([x, y, z]);
    }

    Some(positions)
}

/// Calculate bounding box from all meshes in the GLTF
fn calculate_bounding_box(
    o_blob: &[u8],
    o_json: &gltf::json::Root,
) -> Option<([f32; 3], [f32; 3])> {
    let mut min = [f32::MAX, f32::MAX, f32::MAX];
    let mut max = [f32::MIN, f32::MIN, f32::MIN];
    let mut found_positions = false;

    for mesh in &o_json.meshes {
        for primitive in &mesh.primitives {
            // Look for POSITION attribute
            for (semantic, acc_idx) in &primitive.attributes {
                if matches!(
                    semantic,
                    gltf::json::validation::Checked::Valid(gltf::json::mesh::Semantic::Positions)
                ) && let Some(positions) = get_position_data(o_blob, o_json, *acc_idx)
                {
                    found_positions = true;
                    for pos in positions {
                        min[0] = min[0].min(pos[0]);
                        min[1] = min[1].min(pos[1]);
                        min[2] = min[2].min(pos[2]);
                        max[0] = max[0].max(pos[0]);
                        max[1] = max[1].max(pos[1]);
                        max[2] = max[2].max(pos[2]);
                    }
                }
            }
        }
    }

    if found_positions {
        Some((min, max))
    } else {
        None
    }
}

/// Calculate the offset needed to move pivot to center-bottom
fn calculate_center_bottom_offset(min: [f32; 3], max: [f32; 3]) -> [f32; 3] {
    let center_x = (min[0] + max[0]) / 2.0;
    let center_z = (min[2] + max[2]) / 2.0;
    // Offset is negative because we want to move the model so center-bottom becomes origin
    [-center_x, -min[1], -center_z]
}

pub fn optimize<R: Read + Seek>(
    reader: &mut R,
    new_texture_size: u32,
    remove_normal_texture: bool,
    convert_to_ktx2: bool,
    center_pivot: bool,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let o_data = gltf::Gltf::from_reader(reader)?;
    let o_json = o_data.as_json();
    let o_blob = o_data.blob.as_deref().ok_or("failed to get o_data")?;

    let mut n_blob: Vec<u8> = Vec::new();

    // Calculate pivot offset if requested
    let pivot_offset = if center_pivot {
        calculate_bounding_box(o_blob, o_json)
            .map(|(min, max)| calculate_center_bottom_offset(min, max))
    } else {
        None
    };

    // Clone extensions and add KHR_texture_basisu if not already present
    let mut extensions_required = o_json.extensions_required.clone();
    if convert_to_ktx2 && !extensions_required.contains(&"KHR_texture_basisu".to_string()) {
        extensions_required.push("KHR_texture_basisu".to_string());
    }

    let mut n_json = gltf::json::Root {
        asset: o_json.asset.clone(),
        scene: o_json.scene,
        extensions_required: extensions_required.clone(),
        extensions_used: extensions_required,
        cameras: o_json.cameras.clone(),
        nodes: o_json.nodes.clone(),
        samplers: o_json.samplers.clone(),
        scenes: o_json.scenes.clone(),
        ..Default::default()
    };

    for mesh in o_json.meshes.iter() {
        let mut n_mesh = mesh.clone();
        n_mesh.primitives.clear();
        for p in mesh.primitives.iter() {
            let np = add_primitive(
                &mut n_blob,
                &mut n_json,
                o_blob,
                o_json,
                p,
                new_texture_size,
                remove_normal_texture,
                convert_to_ktx2,
                pivot_offset,
            )?;
            n_mesh.primitives.push(np);
        }

        n_json.push(n_mesh);
    }

    // Process skins and their inverseBindMatrices accessors
    for skin in o_json.skins.iter() {
        let mut n_skin = skin.clone();

        // Copy the inverseBindMatrices accessor if it exists
        if let Some(ibm_idx) = skin.inverse_bind_matrices {
            n_skin.inverse_bind_matrices =
                add_accessor(&mut n_blob, &mut n_json, o_blob, o_json, ibm_idx);
        }

        n_json.push(n_skin);
    }

    pad_to_4bytes(&mut n_blob);

    n_json.push(gltf::json::Buffer {
        byte_length: n_blob.len().into(),
        name: None,
        uri: None,
        extensions: None,
        extras: Default::default(),
    });

    let json_bytes = gltf::json::serialize::to_vec(&n_json)?;

    let n_glb = gltf::binary::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: (json_bytes.len() + n_blob.len()) as u32,
        },
        json: Cow::Owned(json_bytes),
        bin: Some(Cow::Owned(n_blob)),
    };

    let mut result: Vec<u8> = Vec::new();
    let writer = Cursor::new(&mut result);

    n_glb.to_writer(writer)?;

    Ok(result)
}
