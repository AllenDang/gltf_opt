use std::{
    borrow::Cow,
    error::Error,
    io::{Cursor, Read, Seek, Write},
};

use fast_image_resize::IntoImageView;
use gltf::json::{image::MimeType, mesh::Primitive, Index, Root, Texture};
use image::{
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
    ImageEncoder,
};
use ktx2_rw::{BasisCompressionParams, Ktx2Texture};

fn resize_to_jpg<W: Write>(
    img_data: &[u8],
    width: u32,
    height: u32,
    mut buf: W,
) -> Result<(), Box<dyn Error>> {
    let img = image::load_from_memory(img_data)?;

    let mut dst_img =
        fast_image_resize::images::Image::new(width, height, img.pixel_type().unwrap());

    let mut resizer = fast_image_resize::Resizer::new();
    resizer.resize(&img, &mut dst_img, None)?;

    JpegEncoder::new(&mut buf).write_image(dst_img.buffer(), width, height, img.color().into())?;

    Ok(())
}

fn resize_to_png<W: Write>(
    img_data: &[u8],
    width: u32,
    height: u32,
    mut buf: W,
) -> Result<(), Box<dyn Error>> {
    let img = image::load_from_memory(img_data)?;

    let mut dst_img =
        fast_image_resize::images::Image::new(width, height, img.pixel_type().unwrap());

    let mut resizer = fast_image_resize::Resizer::new();
    resizer.resize(&img, &mut dst_img, None)?;

    PngEncoder::new(&mut buf).write_image(dst_img.buffer(), width, height, img.color().into())?;

    Ok(())
}

/// Resize and convert jpeg/png to ktx2 with Basis Universal compression
fn resize_to_ktx2<W: Write>(
    img_data: &[u8],
    width: u32,
    height: u32,
    mut buf: W,
) -> Result<(), Box<dyn Error>> {
    let img = image::load_from_memory(img_data)?;

    let src_img = fast_image_resize::images::Image::from_vec_u8(
        img.width(),
        img.height(),
        img.to_rgba8().into_raw(),
        fast_image_resize::PixelType::U8x4, // Always RGBA8
    )?;

    let mut dst_img =
        fast_image_resize::images::Image::new(width, height, fast_image_resize::PixelType::U8x4);

    let mut resizer = fast_image_resize::Resizer::new();
    resizer.resize(&src_img, &mut dst_img, None)?;

    let mut ktx2_tex = Ktx2Texture::create(width, height, 1, 1, 1, 1, 37)?;
    ktx2_tex.set_image_data(0, 0, 0, dst_img.buffer())?;
    ktx2_tex.set_metadata("Tool", b"glb_opt")?;
    ktx2_tex.set_metadata("Dimensions", format!("{width}x{height}").as_bytes())?;

    let etc1s_params = BasisCompressionParams::builder()
        .uastc(false)
        .thread_count(num_cpus::get() as u32)
        .quality_level(224)
        .endpoint_rdo_threshold(1.25)
        .selector_rdo_threshold(1.25)
        .build();
    ktx2_tex.compress_basis(&etc1s_params)?;
    ktx2_tex.set_metadata("CompressionMode", b"ETC1S")?;

    let ktx2_data = ktx2_tex.write_to_memory()?;
    buf.write_all(&ktx2_data)?;

    Ok(())
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

    n_json.push(n_img)
}

fn add_accessor(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    idx: Index<gltf::json::Accessor>,
) -> Option<Index<gltf::json::Accessor>> {
    if let Some(acc) = o_json.accessors.get(idx.value()) {
        if let Some(idx_view) = acc.buffer_view {
            if let Some(view) = o_json.buffer_views.get(idx_view.value()) {
                let offset = match view.byte_offset {
                    Some(o) => o.0 as usize,
                    None => 0,
                };
                let length = view.byte_length.0 as usize;

                if let Some(data) = &o_blob.get(offset..(offset + length)) {
                    let offset = n_blob.len();
                    let length = data.len();

                    n_blob.extend_from_slice(data);

                    // create buffer_view
                    let mut n_view = view.clone();
                    n_view.byte_offset = Some(offset.into());
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
    }

    None
}

fn get_image_data<'a>(
    o_blob: &'a [u8],
    o_json: &gltf::json::Root,
    texture_idx: Index<Texture>,
) -> Option<&'a [u8]> {
    if let Some(tex) = o_json.textures.get(texture_idx.value()) {
        if let Some(img) = o_json.images.get(tex.source.value()) {
            if let Some(idx_view) = img.buffer_view {
                if let Some(view) = o_json.buffer_views.get(idx_view.value()) {
                    let offset = match view.byte_offset {
                        Some(o) => o.0 as usize,
                        None => 0,
                    };
                    let length = view.byte_length.0 as usize;

                    return o_blob.get(offset..(offset + length));
                }
            }
        }
    }
    None
}

fn add_texture(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    info: &gltf::json::texture::Info,
    n_tex_size: u32,
    convert_to_ktx2: bool,
) -> Option<gltf::json::texture::Info> {
    if let Some(bct_image_data) = get_image_data(o_blob, o_json, info.index) {
        let mut new_bct_data: Vec<u8> = Vec::new();
        let mut writer = Cursor::new(&mut new_bct_data);

        let mime_type = if convert_to_ktx2 {
            "image/ktx2"
        } else {
            "image/jpeg"
        };

        let resize_func = if convert_to_ktx2 {
            resize_to_ktx2
        } else {
            resize_to_jpg
        };

        return match resize_func(bct_image_data, n_tex_size, n_tex_size, &mut writer) {
            Ok(_) => {
                let new_image = o_json.images.get(info.index.value()).unwrap().clone();

                let idx_img = add_image(
                    n_blob,
                    n_json,
                    &new_image,
                    &new_bct_data.to_vec(),
                    mime_type,
                );

                // create new texture
                let mut new_tex = o_json.textures.get(info.index.value()).unwrap().clone();
                new_tex.source = idx_img;

                let idx_tex = n_json.push(new_tex);

                let mut new_info = info.clone();
                new_info.index = idx_tex;

                Some(new_info)
            }
            Err(_) => None,
        };
    }
    None
}

fn add_normal_texture(
    n_blob: &mut Vec<u8>,
    n_json: &mut Root,
    o_blob: &[u8],
    o_json: &gltf::json::Root,
    normal: &gltf::json::material::NormalTexture,
    n_tex_size: u32,
    convert_to_ktx2: bool,
) -> Option<gltf::json::material::NormalTexture> {
    if let Some(bct_image_data) = get_image_data(o_blob, o_json, normal.index) {
        let mut new_bct_data: Vec<u8> = Vec::new();
        let mut writer = Cursor::new(&mut new_bct_data);

        let mime_type = if convert_to_ktx2 {
            "image/ktx2"
        } else {
            "image/png"
        };

        let resize_func = if convert_to_ktx2 {
            resize_to_ktx2
        } else {
            resize_to_png
        };

        return match resize_func(bct_image_data, n_tex_size, n_tex_size, &mut writer) {
            Ok(_) => {
                let new_image = o_json.images.get(normal.index.value()).unwrap().clone();

                let idx_img = add_image(
                    n_blob,
                    n_json,
                    &new_image,
                    &new_bct_data.to_vec(),
                    mime_type,
                );

                // create new texture
                let mut new_tex = o_json.textures.get(normal.index.value()).unwrap().clone();
                new_tex.source = idx_img;

                let idx_tex = n_json.push(new_tex);

                let mut new_normal = normal.clone();
                new_normal.index = idx_tex;

                Some(new_normal)
            }
            Err(_) => None,
        };
    }

    None
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
) -> Primitive {
    let mut n_p = p.clone();

    // copy indices
    if let Some(indices) = p.indices {
        n_p.indices = add_accessor(n_blob, n_json, o_blob, o_json, indices);
    }

    // copy attributes
    n_p.attributes.clear();
    for (k, v) in &p.attributes {
        if let Some(idx_acc) = add_accessor(n_blob, n_json, o_blob, o_json, *v) {
            n_p.attributes.insert(k.clone(), idx_acc);
        }
    }

    // add material
    if let Some(idx_mat) = p.material {
        if let Some(mat) = o_json.materials.get(idx_mat.value()) {
            let mut n_mat = mat.clone();

            // resize base color tex
            if let Some(bct_info) = &mat.pbr_metallic_roughness.base_color_texture {
                if let Some(new_info) = add_texture(
                    n_blob,
                    n_json,
                    o_blob,
                    o_json,
                    bct_info,
                    n_tex_size,
                    convert_to_ktx2,
                ) {
                    n_mat.pbr_metallic_roughness.base_color_texture = Some(new_info);
                }
            }

            // resize metal/rough tex
            if let Some(mr_info) = &mat.pbr_metallic_roughness.metallic_roughness_texture {
                if let Some(new_info) = add_texture(
                    n_blob,
                    n_json,
                    o_blob,
                    o_json,
                    mr_info,
                    n_tex_size / 2,
                    convert_to_ktx2,
                ) {
                    n_mat.pbr_metallic_roughness.metallic_roughness_texture = Some(new_info);
                }
            }

            if remove_normal_texture {
                n_mat.normal_texture = None;
            } else {
                // resize normal map
                if let Some(normal_tex) = &mat.normal_texture {
                    n_mat.normal_texture = add_normal_texture(
                        n_blob,
                        n_json,
                        o_blob,
                        o_json,
                        normal_tex,
                        n_tex_size,
                        convert_to_ktx2,
                    );
                }
            }

            // update material
            let idx_mat = n_json.push(n_mat);
            n_p.material = Some(idx_mat);
        }
    }

    n_p
}

fn pad_to_4bytes(data: &mut Vec<u8>) {
    while data.len() % 4 != 0 {
        data.push(0);
    }
}

pub fn optimize<R: Read + Seek>(
    reader: &mut R,
    new_texture_size: u32,
    remove_normal_texture: bool,
    convert_to_ktx2: bool,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let o_data = gltf::Gltf::from_reader(reader)?;
    let o_json = o_data.as_json();
    let o_blob = o_data.blob.as_deref().unwrap();

    let mut n_blob: Vec<u8> = Vec::new();

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
        skins: o_json.skins.clone(),
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
            );
            n_mesh.primitives.push(np);
        }

        n_json.push(n_mesh);
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
