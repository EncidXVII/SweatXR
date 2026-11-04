//! Overlay textures through Vulkan. One device-local image per overlay, updated in place from a
//! persistently mapped staging buffer, so SteamVR allocates nothing per frame. (SetOverlayRaw
//! leaked ~650 KB per upload and failed after ~240 uploads in the soak test.)

use std::ffi::{CString, c_void};

use ash::vk::{self, Handle};

use super::{VrError, VulkanTextureData};

/// Colours are sRGB-encoded bytes; an sRGB image lets the compositor blend them correctly.
const FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;

fn vk_err(call: &'static str) -> impl Fn(vk::Result) -> VrError {
    move |r| VrError {
        call,
        code: r.as_raw() as u32,
        detail: format!("{r:?}"),
    }
}

struct Slot {
    image: vk::Image,
    image_mem: vk::DeviceMemory,
    staging: vk::Buffer,
    staging_mem: vk::DeviceMemory,
    mapped: *mut u8,
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    width: u32,
    height: u32,
    initialised: bool,
}

pub struct Gpu {
    _entry: ash::Entry,
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    pool: vk::CommandPool,
    slots: Vec<Slot>,
}

impl Gpu {
    pub(super) fn new(
        instance_ext: &[CString],
        output_device: impl FnOnce(*mut c_void) -> u64,
        device_ext: impl FnOnce(*mut c_void) -> Vec<CString>,
    ) -> Result<Self, VrError> {
        // SAFETY: loads the system Vulkan loader; every call below follows the Vulkan spec's
        // valid-usage rules for these simple objects, and all handles are destroyed in Drop.
        unsafe {
            let entry = ash::Entry::load().map_err(|e| VrError {
                call: "load Vulkan",
                code: 0,
                detail: e.to_string(),
            })?;
            let app = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 1, 0));
            let ext: Vec<_> = instance_ext.iter().map(|e| e.as_ptr()).collect();
            let instance = entry
                .create_instance(
                    &vk::InstanceCreateInfo::default()
                        .application_info(&app)
                        .enabled_extension_names(&ext),
                    None,
                )
                .map_err(vk_err("vkCreateInstance"))?;

            // Use the GPU the headset is on; fall back to the first one.
            let wanted = output_device(instance.handle().as_raw() as *mut c_void);
            let devices = instance
                .enumerate_physical_devices()
                .map_err(vk_err("vkEnumeratePhysicalDevices"))?;
            let physical = devices
                .iter()
                .copied()
                .find(|d| d.as_raw() == wanted)
                .or(devices.first().copied());
            let Some(physical) = physical else {
                instance.destroy_instance(None);
                return Err(VrError {
                    call: "Vulkan",
                    code: 0,
                    detail: "no GPU".into(),
                });
            };
            let family = instance
                .get_physical_device_queue_family_properties(physical)
                .iter()
                .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                .map(|i| i as u32);
            let Some(family) = family else {
                instance.destroy_instance(None);
                return Err(VrError {
                    call: "Vulkan",
                    code: 0,
                    detail: "no graphics queue".into(),
                });
            };
            let dev_ext = device_ext(physical.as_raw() as *mut c_void);
            let dev_ext: Vec<_> = dev_ext.iter().map(|e| e.as_ptr()).collect();
            let queue_info = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&[1.0])];
            let device = match instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queue_info)
                    .enabled_extension_names(&dev_ext),
                None,
            ) {
                Ok(d) => d,
                Err(r) => {
                    instance.destroy_instance(None);
                    return Err(vk_err("vkCreateDevice")(r));
                }
            };
            let queue = device.get_device_queue(family, 0);
            let pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .map_err(vk_err("vkCreateCommandPool"));
            let mut gpu = Self {
                _entry: entry,
                instance,
                physical,
                device,
                queue,
                family,
                pool: vk::CommandPool::null(),
                slots: Vec::new(),
            };
            // From here Drop cleans up whatever exists.
            gpu.pool = pool?;
            Ok(gpu)
        }
    }

    fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32, VrError> {
        // SAFETY: `physical` belongs to `instance`.
        let props = unsafe { self.instance.get_physical_device_memory_properties(self.physical) };
        (0..props.memory_type_count)
            .find(|&i| bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(flags))
            .ok_or(VrError {
                call: "Vulkan",
                code: 0,
                detail: "no suitable memory".into(),
            })
    }

    /// Adds a texture of this size and returns its slot.
    pub fn add_texture(&mut self, width: u32, height: u32) -> Result<usize, VrError> {
        // SAFETY: standard image + buffer + memory setup on our own device. On error the
        // partly built objects are pushed as a slot anyway so Drop frees them.
        unsafe {
            let d = &self.device;
            let mut slot = Slot {
                image: vk::Image::null(),
                image_mem: vk::DeviceMemory::null(),
                staging: vk::Buffer::null(),
                staging_mem: vk::DeviceMemory::null(),
                mapped: std::ptr::null_mut(),
                cmd: vk::CommandBuffer::null(),
                fence: vk::Fence::null(),
                width,
                height,
                initialised: false,
            };
            let result = (|| -> Result<(), VrError> {
                slot.image = d
                    .create_image(
                        &vk::ImageCreateInfo::default()
                            .image_type(vk::ImageType::TYPE_2D)
                            .format(FORMAT)
                            .extent(vk::Extent3D {
                                width,
                                height,
                                depth: 1,
                            })
                            .mip_levels(1)
                            .array_layers(1)
                            .samples(vk::SampleCountFlags::TYPE_1)
                            .tiling(vk::ImageTiling::OPTIMAL)
                            .usage(
                                vk::ImageUsageFlags::TRANSFER_DST
                                    | vk::ImageUsageFlags::TRANSFER_SRC
                                    | vk::ImageUsageFlags::SAMPLED,
                            ),
                        None,
                    )
                    .map_err(vk_err("vkCreateImage"))?;
                let req = d.get_image_memory_requirements(slot.image);
                let ty = self.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)?;
                slot.image_mem = d
                    .allocate_memory(
                        &vk::MemoryAllocateInfo::default()
                            .allocation_size(req.size)
                            .memory_type_index(ty),
                        None,
                    )
                    .map_err(vk_err("vkAllocateMemory"))?;
                d.bind_image_memory(slot.image, slot.image_mem, 0)
                    .map_err(vk_err("vkBindImageMemory"))?;

                let size = u64::from(width) * u64::from(height) * 4;
                slot.staging = d
                    .create_buffer(
                        &vk::BufferCreateInfo::default()
                            .size(size)
                            .usage(vk::BufferUsageFlags::TRANSFER_SRC),
                        None,
                    )
                    .map_err(vk_err("vkCreateBuffer"))?;
                let req = d.get_buffer_memory_requirements(slot.staging);
                let host = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
                let ty = self.memory_type(req.memory_type_bits, host)?;
                slot.staging_mem = d
                    .allocate_memory(
                        &vk::MemoryAllocateInfo::default()
                            .allocation_size(req.size)
                            .memory_type_index(ty),
                        None,
                    )
                    .map_err(vk_err("vkAllocateMemory"))?;
                d.bind_buffer_memory(slot.staging, slot.staging_mem, 0)
                    .map_err(vk_err("vkBindBufferMemory"))?;
                slot.mapped = d
                    .map_memory(slot.staging_mem, 0, size, vk::MemoryMapFlags::empty())
                    .map_err(vk_err("vkMapMemory"))?
                    .cast();

                slot.cmd = d
                    .allocate_command_buffers(
                        &vk::CommandBufferAllocateInfo::default()
                            .command_pool(self.pool)
                            .level(vk::CommandBufferLevel::PRIMARY)
                            .command_buffer_count(1),
                    )
                    .map_err(vk_err("vkAllocateCommandBuffers"))?[0];
                slot.fence = d
                    .create_fence(
                        &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                        None,
                    )
                    .map_err(vk_err("vkCreateFence"))?;
                Ok(())
            })();
            self.slots.push(slot);
            result.map(|()| self.slots.len() - 1)
        }
    }

    /// Copies `rgba` into the slot's image, waits for the copy, and returns what SteamVR needs.
    pub(super) fn upload(&mut self, slot: usize, rgba: &[u8]) -> Result<VulkanTextureData, VrError> {
        let s = self.slots.get_mut(slot).ok_or(VrError {
            call: "upload",
            code: 0,
            detail: "no such texture".into(),
        })?;
        if rgba.len() != (s.width * s.height * 4) as usize {
            return Err(VrError {
                call: "upload",
                code: 0,
                detail: "buffer size".into(),
            });
        }
        let d = &self.device;
        // SAFETY: the fence wait guarantees the previous copy finished before the staging
        // memory is overwritten; `mapped` covers exactly width * height * 4 bytes.
        unsafe {
            d.wait_for_fences(&[s.fence], true, u64::MAX)
                .map_err(vk_err("vkWaitForFences"))?;
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), s.mapped, rgba.len());
            d.reset_fences(&[s.fence]).map_err(vk_err("vkResetFences"))?;
            d.begin_command_buffer(
                s.cmd,
                &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(vk_err("vkBeginCommandBuffer"))?;
            let range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1);
            let barrier = |from, to, src, dst| {
                vk::ImageMemoryBarrier::default()
                    .old_layout(from)
                    .new_layout(to)
                    .src_access_mask(src)
                    .dst_access_mask(dst)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(s.image)
                    .subresource_range(range)
            };
            let before = if s.initialised {
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL
            } else {
                vk::ImageLayout::UNDEFINED
            };
            d.cmd_pipeline_barrier(
                s.cmd,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier(
                    before,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::AccessFlags::empty(),
                    vk::AccessFlags::TRANSFER_WRITE,
                )],
            );
            let region = vk::BufferImageCopy::default()
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .layer_count(1),
                )
                .image_extent(vk::Extent3D {
                    width: s.width,
                    height: s.height,
                    depth: 1,
                });
            d.cmd_copy_buffer_to_image(
                s.cmd,
                s.staging,
                s.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
            // SteamVR requires TRANSFER_SRC_OPTIMAL when it reads the texture.
            d.cmd_pipeline_barrier(
                s.cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier(
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::AccessFlags::TRANSFER_WRITE,
                    vk::AccessFlags::TRANSFER_READ,
                )],
            );
            d.end_command_buffer(s.cmd)
                .map_err(vk_err("vkEndCommandBuffer"))?;
            let cmds = [s.cmd];
            d.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&cmds)],
                s.fence,
            )
            .map_err(vk_err("vkQueueSubmit"))?;
            d.wait_for_fences(&[s.fence], true, u64::MAX)
                .map_err(vk_err("vkWaitForFences"))?;
        }
        s.initialised = true;
        Ok(VulkanTextureData {
            image: s.image.as_raw(),
            device: self.device.handle().as_raw() as *mut c_void,
            physical_device: self.physical.as_raw() as *mut c_void,
            instance: self.instance.handle().as_raw() as *mut c_void,
            queue: self.queue.as_raw() as *mut c_void,
            queue_family_index: self.family,
            width: s.width,
            height: s.height,
            format: FORMAT.as_raw() as u32,
            sample_count: 1,
        })
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: waits for all GPU work, then destroys children before their parents; null
        // handles from a half-built slot are valid no-ops for vkDestroy*/vkFree*.
        unsafe {
            let d = &self.device;
            let _ = d.device_wait_idle();
            for s in self.slots.drain(..) {
                d.destroy_fence(s.fence, None);
                d.destroy_buffer(s.staging, None);
                d.free_memory(s.staging_mem, None);
                d.destroy_image(s.image, None);
                d.free_memory(s.image_mem, None);
            }
            d.destroy_command_pool(self.pool, None);
            d.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
