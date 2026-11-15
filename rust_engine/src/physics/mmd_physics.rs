//! MMD 物理世界管理器
//!
//! 移植自 babylon-mmd，使用 Bullet3 引擎。
//! 实现 commitBodyStates / syncBodies / stepSimulation / syncBones 全套流程。

use std::collections::{HashMap, HashSet};

use glam::{Mat3, Mat4, Quat, Vec3};

use mmd::pmx::joint::Joint as PmxJoint;
use mmd::pmx::rigid_body::RigidBody as PmxRigidBody;

use super::bullet_ffi::{
    self, BulletConstraint, BulletRigidBody, BulletShape, BulletWorld, RigidBodyInfo,
    BT_CONSTRAINT_STOP_ERP,
};
use super::config::{get_config, PhysicsConfig};
use super::mmd_joint::MmdJointData;
use super::mmd_rigid_body::{MmdRigidBodyData, PhysicsMode};

const MAX_RAGDOLL_TERRAIN_BOXES: usize = 256;
const MAX_RAGDOLL_ACTORS: usize = 16;
const RAGDOLL_ACTOR_COLLISION_GROUP: i32 = 0x4000;
const RAGDOLL_CORE_TAG: i32 = 1;
const RAGDOLL_ACTOR_TAG: i32 = 2;

/// The skinning update order matters after death: synthetic rig roots move
/// first, articulated bodies second, and PMX rotation-only accessories last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicsBoneUpdate {
    RagdollRoot,
    Dynamic,
    RotationOnly,
}

/// The body must be dropped before its collision shape. Neither is owned by
/// BulletWorld; we detach the body before Rust drops either one.
struct TerrainCollider {
    body: BulletRigidBody,
    _shape: BulletShape,
}

/// Kinematic collision proxy for a nearby Minecraft player or mob. Retaining
/// it across ticks lets Bullet derive motion and push the dynamic ragdoll.
struct ActorCollider {
    entity_id: i32,
    half_extents: Vec3,
    body: BulletRigidBody,
    _shape: BulletShape,
}

/// MMD 物理世界管理器（Bullet3 引擎）
///
/// 移植自 babylon-mmd，管理 Bullet3 世界、刚体、关节。
/// 物理在模型局部空间运行（Minecraft 仅绕 Y 轴旋转，重力方向不变）。
/// 流程：build_physics → 每帧 [sync_bodies → stepSimulation → sync_bones]
pub struct MMDPhysics {
    /// MMD 关节数据列表（Drop 顺序：约束先于刚体先于世界）
    joints: Vec<MmdJointData>,
    /// MMD 刚体数据列表
    pub rigid_bodies: Vec<MmdRigidBodyData>,
    /// Emergency floor until Minecraft collision shapes have been sampled.
    ragdoll_floor_body: Option<BulletRigidBody>,
    ragdoll_floor_shape: Option<BulletShape>,
    /// Snapshot of nearby Minecraft block collision boxes in PMX model space.
    ragdoll_terrain: Vec<TerrainCollider>,
    /// Moving Minecraft entities, driven by 20 Hz client AABB snapshots.
    ragdoll_actors: Vec<ActorCollider>,
    /// Bullet3 物理世界
    world: BulletWorld,
    /// 物理 FPS
    fps: f32,
    /// 最大子步数
    max_substep_count: i32,

    // --- 预分配缓冲区（避免每帧堆分配） ---
    /// 动态刚体关联的骨骼索引集合（构建时计算一次）
    dynamic_bone_indices: HashSet<usize>,
    /// Bones whose world positions (not only rotation) come from Bullet.
    position_driven_bone_indices: HashSet<usize>,
    /// Virtual control bone(s) that move with a dynamic body but have no PMX collider.
    ragdoll_root_bone_indices: HashSet<usize>,
    /// (control bone, anatomical rigid body, death-pose control-to-body offset)
    ragdoll_root_anchor: Option<(usize, usize, Mat4)>,
    /// 动态骨骼变换结果缓冲区（复用内存）
    dynamic_bone_buf: Vec<(usize, Mat4, PhysicsBoneUpdate)>,

    /// 上一帧模型世界位置（用于计算移动速度实现惯性）
    prev_model_position: Option<Vec3>,
}

impl MMDPhysics {
    /// 创建新的物理世界（C++ OOM 时返回 None）
    pub fn new() -> Option<Self> {
        let config = get_config();
        let world = BulletWorld::new(0.0, config.gravity_y, 0.0)?;

        if config.debug_log {
            log::info!(
                "[Bullet3] 物理世界创建: FPS={}, 重力Y={}",
                config.physics_fps,
                config.gravity_y
            );
        }

        Some(Self {
            joints: Vec::new(),
            rigid_bodies: Vec::new(),
            ragdoll_floor_body: None,
            ragdoll_floor_shape: None,
            ragdoll_terrain: Vec::new(),
            ragdoll_actors: Vec::new(),
            world,
            fps: config.physics_fps,
            max_substep_count: config.max_substep_count,
            dynamic_bone_indices: HashSet::new(),
            position_driven_bone_indices: HashSet::new(),
            ragdoll_root_bone_indices: HashSet::new(),
            ragdoll_root_anchor: None,
            dynamic_bone_buf: Vec::new(),
            prev_model_position: None,
        })
    }

    /// 构建物理系统（移植自 babylon-mmd buildPhysics）
    ///
    /// 一次性创建所有刚体和关节，并添加到 Bullet3 世界。
    /// 先将对象存入 Vec（所有权转移），再添加到世界，保证 panic 安全。
    pub fn build_physics(
        &mut self,
        pmx_rigid_bodies: &[PmxRigidBody],
        pmx_joints: &[PmxJoint],
        bone_transforms: &[Mat4],
    ) {
        let config = get_config();

        // 预分配容量
        self.rigid_bodies.reserve(pmx_rigid_bodies.len());

        // 第一步：创建所有刚体并存入 Vec（还未加入世界）
        for pmx_rb in pmx_rigid_bodies {
            let bone_transform =
                if pmx_rb.bone_index >= 0 && (pmx_rb.bone_index as usize) < bone_transforms.len() {
                    Some(bone_transforms[pmx_rb.bone_index as usize])
                } else {
                    None
                };

            let mut rb_data = MmdRigidBodyData::from_pmx(pmx_rb, bone_transform);
            let shape = MmdRigidBodyData::create_shape(pmx_rb);
            let body = shape
                .as_ref()
                .and_then(|s| rb_data.create_rigid_body(pmx_rb, s));

            if shape.is_none() || body.is_none() {
                log::warn!("[Bullet3] 刚体 '{}' 创建失败，跳过", rb_data.name);
            }

            // 先转移所有权到 rb_data，再 push 到 Vec
            rb_data.bullet_body = body;
            rb_data.bullet_shape = shape;
            self.rigid_bodies.push(rb_data);
        }

        // 第二步：统一将已存储的刚体添加到世界
        // 此时所有权已在 self.rigid_bodies 中，panic 时 Drop 链会正确清理
        for rb_data in &self.rigid_bodies {
            if let Some(ref body) = rb_data.bullet_body {
                let group = 1i32 << (rb_data.group.min(15) as i32);
                let mask = rb_data.group_mask as i32;
                self.world.add_rigid_body(body, group, mask);
            }
        }

        // 第三步：创建关节并存入 Vec
        if config.joints_enabled {
            self.joints.reserve(pmx_joints.len());

            for pmx_joint in pmx_joints {
                let rb_a_idx = pmx_joint.rigid_body_a_index as usize;
                let rb_b_idx = pmx_joint.rigid_body_b_index as usize;

                if rb_a_idx >= self.rigid_bodies.len()
                    || rb_b_idx >= self.rigid_bodies.len()
                    || rb_a_idx == rb_b_idx
                {
                    continue;
                }

                let (rb_a_body, rb_b_body, rb_a_init, rb_b_init) = {
                    let rb_a = &self.rigid_bodies[rb_a_idx];
                    let rb_b = &self.rigid_bodies[rb_b_idx];
                    match (&rb_a.bullet_body, &rb_b.bullet_body) {
                        (Some(a), Some(b)) => {
                            (a, b, rb_a.initial_transform, rb_b.initial_transform)
                        }
                        _ => continue,
                    }
                };

                let joint_data =
                    MmdJointData::from_pmx(pmx_joint, rb_a_body, rb_b_body, rb_a_init, rb_b_init);

                // 先存入 Vec，再添加到世界
                self.joints.push(joint_data);
            }

            // 统一添加约束到世界
            for joint_data in &self.joints {
                if let Some(ref constraint) = joint_data.constraint {
                    self.world.add_constraint(constraint, true);
                }
            }
        }

        // 第四步：预计算动态骨骼索引集合（一次性，避免每帧重算）
        self.dynamic_bone_indices = self
            .rigid_bodies
            .iter()
            .filter(|rb| rb.physics_mode != PhysicsMode::FollowBone && rb.bone_index >= 0)
            .map(|rb| rb.bone_index as usize)
            .collect();
        self.position_driven_bone_indices = self
            .rigid_bodies
            .iter()
            .filter(|rb| rb.physics_mode == PhysicsMode::Physics && rb.bone_index >= 0)
            .map(|rb| rb.bone_index as usize)
            .collect();

        // 预分配动态骨骼缓冲区
        self.dynamic_bone_buf
            .reserve(self.dynamic_bone_indices.len());

        self.world.set_kinematic_filter(config.kinematic_filter);

        let (kinematic_count, dynamic_count, dynamic_bone_count) =
            self.rigid_bodies
                .iter()
                .fold((0, 0, 0), |(k, d, db), rb| match rb.physics_mode {
                    PhysicsMode::FollowBone => (k + 1, d, db),
                    PhysicsMode::Physics => (k, d + 1, db),
                    PhysicsMode::PhysicsWithBone => (k, d, db + 1),
                });

        log::info!(
            "Bullet3 物理构建完成: {} 刚体 ({}跟骨 + {}物理 + {}物理跟骨), {} 关节",
            self.rigid_bodies.len(),
            kinematic_count,
            dynamic_count,
            dynamic_bone_count,
            self.joints.len()
        );
    }

    /// 同步运动学刚体位置（babylon-mmd syncBodies）
    ///
    /// 在每帧物理步进前调用。将 FollowBone 模式的刚体位置
    /// 同步为骨骼变换推导的物理空间（左手）变换。
    pub fn sync_bodies(&self, bone_transforms: &[Mat4]) {
        for rb_data in &self.rigid_bodies {
            if rb_data.physics_mode != PhysicsMode::FollowBone {
                continue;
            }
            let bone_idx = rb_data.bone_index;
            if bone_idx < 0 || (bone_idx as usize) >= bone_transforms.len() {
                continue;
            }
            if let Some(ref body) = rb_data.bullet_body {
                // 骨骼(右手) → inv_z → 左手，再计算刚体位置
                let bone_left = super::inv_z(bone_transforms[bone_idx as usize]);
                let body_matrix = rb_data.compute_body_matrix(bone_left);
                body.set_transform(body_matrix);
            }
        }
    }

    /// 同步运动学刚体并传递模型移动速度（实现惯性）
    ///
    /// 原理：物理在模型局部空间运行，角色世界移动对物理不可见。
    /// 第一步：同步 FollowBone 刚体到骨骼位置。
    /// 第二步：计算模型世界速度，转换到物理空间后取反，
    ///        对动态刚体施加 F = m * (-v_local) / dt，
    ///        产生与移动方向相反的惯性力（头发/裙子自然后拽）。
    pub fn sync_bodies_with_model_velocity(
        &mut self,
        bone_transforms: &[Mat4],
        delta_time: f32,
        model_transform: Mat4,
        config: &PhysicsConfig,
    ) {
        let dt = delta_time.max(0.001);
        let curr_pos = model_transform.w_axis.truncate();

        // 计算模型世界速度
        let model_velocity = if let Some(prev_pos) = self.prev_model_position {
            (curr_pos - prev_pos) / dt
        } else {
            Vec3::ZERO
        };
        self.prev_model_position = Some(curr_pos);

        // 第一步：同步运动学刚体位置
        self.sync_bodies(bone_transforms);

        // 第二步：给动态刚体施加惯性力
        if config.inertia_strength > 0.0 && model_velocity.length_squared() > 1e-8 {
            // 钳制世界速度（20 blocks/s 覆盖疾跑+速度药水，超出视为传送）
            let speed_sq = model_velocity.length_squared();
            let max_speed = 20.0_f32;
            let world_vel = if speed_sq > max_speed * max_speed {
                model_velocity * (max_speed / speed_sq.sqrt())
            } else {
                model_velocity
            };

            // 世界速度 → 模型局部空间（R^T * v_world）
            let rot_inv = Mat3::from_mat4(model_transform).transpose();
            let local_vel = rot_inv * world_vel;

            // 模型局部空间惯性（Z-flip 坐标系中 Z 方向需取反）
            let inertia_vel = Vec3::new(
                -local_vel.x * config.inertia_strength,
                -local_vel.y * config.inertia_strength,
                local_vel.z * config.inertia_strength,
            );

            // 最大加速度 = max_linear_velocity * physics_fps
            // 确保单个物理子步内速度增量不超过 max_linear_velocity
            let max_accel = config.max_linear_velocity * self.fps;

            // F = m * v / dt，再钳制力大小防止衣服拉伸
            for rb_data in &self.rigid_bodies {
                if rb_data.physics_mode == PhysicsMode::FollowBone {
                    continue;
                }
                if let Some(ref body) = rb_data.bullet_body {
                    let mass = body.get_mass();
                    if mass > 0.0 {
                        let mut force = inertia_vel * mass / dt;
                        // 钳制力：限制加速度上限
                        let max_force = max_accel * mass;
                        let force_sq = force.length_squared();
                        if force_sq > max_force * max_force {
                            force *= max_force / force_sq.sqrt();
                        }
                        body.apply_central_force(force.x, force.y, force.z);
                    }
                }
            }
        }
    }

    /// 步进物理模拟（Bullet3 stepSimulation）+ 速度钳制
    ///
    /// Bullet3 没有内置全局速度限制，需在每步后手动截断超速刚体，
    /// 防止卡顿帧或极端力导致的物理爆炸。
    pub fn step_simulation(&self, delta_time: f32, config: &PhysicsConfig) {
        let fixed_dt = 1.0 / self.fps;
        self.world
            .step(delta_time, self.max_substep_count, fixed_dt);

        // 速度钳制
        let max_lin = config.max_linear_velocity;
        let max_ang = config.max_angular_velocity;
        let max_lin_sq = max_lin * max_lin;
        let max_ang_sq = max_ang * max_ang;

        for rb_data in &self.rigid_bodies {
            if rb_data.physics_mode == PhysicsMode::FollowBone {
                continue;
            }
            if let Some(ref body) = rb_data.bullet_body {
                let lin_vel = body.get_linear_velocity();
                let lin_sq = lin_vel.length_squared();
                if lin_sq > max_lin_sq {
                    let clamped = lin_vel * (max_lin / lin_sq.sqrt());
                    body.set_linear_velocity(clamped.x, clamped.y, clamped.z);
                }

                let ang_vel = body.get_angular_velocity();
                let ang_sq = ang_vel.length_squared();
                if ang_sq > max_ang_sq {
                    let clamped = ang_vel * (max_ang / ang_sq.sqrt());
                    body.set_angular_velocity(clamped.x, clamped.y, clamped.z);
                }
            }
        }
    }

    /// 将动态刚体变换同步回骨骼（babylon-mmd syncBones）
    ///
    /// 从 Bullet3 读取左手空间变换，通过 inv_z 转回右手空间写入骨骼。
    pub fn sync_bones(&self, bone_transforms: &mut [Mat4]) {
        for rb_data in &self.rigid_bodies {
            if rb_data.physics_mode == PhysicsMode::FollowBone {
                continue;
            }
            let bone_idx = rb_data.bone_index;
            if bone_idx < 0 || (bone_idx as usize) >= bone_transforms.len() {
                continue;
            }
            if let Some(ref body) = rb_data.bullet_body {
                let rb_matrix = body.get_transform();
                let new_bone_left = match rb_data.physics_mode {
                    PhysicsMode::Physics => rb_data.compute_bone_matrix(rb_matrix),
                    PhysicsMode::PhysicsWithBone => {
                        // 骨骼位置(右手) → inv_z → 左手
                        let bone_right = bone_transforms[bone_idx as usize];
                        let pos_left = Vec3::new(
                            bone_right.w_axis.x,
                            bone_right.w_axis.y,
                            -bone_right.w_axis.z,
                        );
                        rb_data.compute_bone_matrix_rotation_only(rb_matrix, pos_left)
                    }
                    PhysicsMode::FollowBone => unreachable!(),
                };
                // 左手 → inv_z → 右手
                bone_transforms[bone_idx as usize] = super::inv_z(new_bone_left);
            }
        }
    }

    /// 初始化物理（commitBodyStates 的初始版本）
    ///
    /// 在骨骼初始姿态确定后调用，将所有刚体设置到正确的初始位置（左手空间）。
    pub fn initialize(&mut self, bone_transforms: &[Mat4]) {
        for rb_data in &self.rigid_bodies {
            let bone_idx = rb_data.bone_index;
            if bone_idx < 0 || (bone_idx as usize) >= bone_transforms.len() {
                continue;
            }
            if let Some(ref body) = rb_data.bullet_body {
                // 骨骼(右手) → inv_z → 左手
                let bone_left = super::inv_z(bone_transforms[bone_idx as usize]);
                let body_matrix = rb_data.compute_body_matrix(bone_left);
                body.set_transform(body_matrix);
                body.set_linear_velocity(0.0, 0.0, 0.0);
                body.set_angular_velocity(0.0, 0.0, 0.0);
                body.clear_forces();
            }
        }
    }

    /// Convert existing bone-following collision bodies into an articulated
    /// ragdoll without restarting the secondary (hair/skirt/tail) simulation.
    /// Call once after the last living animation frame has been evaluated.
    pub fn start_ragdoll(
        &mut self,
        bone_parents: &[i32],
        bone_transforms: &[Mat4],
    ) -> bool {
        let original_kinematic: Vec<(usize, usize)> = self
            .rigid_bodies
            .iter()
            .enumerate()
            .filter_map(|(rb_idx, rb)| {
                let bone = usize::try_from(rb.bone_index).ok()?;
                if rb.physics_mode == PhysicsMode::FollowBone
                    && bone < bone_transforms.len()
                    && rb.bullet_body.is_some()
                {
                    Some((rb_idx, bone))
                } else {
                    None
                }
            })
            .collect();

        // Don't turn hair-only models into free-floating physics objects.
        if original_kinematic.len() < 3 {
            return false;
        }

        // Snap the kinematic bodies to the exact death pose before changing
        // their collision and mass flags. The original PMX may set mass=0.
        for &(rb_idx, bone_idx) in &original_kinematic {
            let rb = &self.rigid_bodies[rb_idx];
            if let Some(ref body) = rb.bullet_body {
                let bone_left = super::inv_z(bone_transforms[bone_idx]);
                body.set_transform(rb.compute_body_matrix(bone_left));
                body.set_linear_velocity(0.0, 0.0, 0.0);
                body.set_angular_velocity(0.0, 0.0, 0.0);
            }
        }

        // Re-register bodies to refresh Bullet's broadphase classification.
        // Collision bit 15 is reserved for the prototype floor.
        for rb in &mut self.rigid_bodies {
            let Some(ref body) = rb.bullet_body else { continue };
            self.world.remove_rigid_body(body);
            if rb.physics_mode == PhysicsMode::FollowBone
                && rb.bone_index >= 0
                && (rb.bone_index as usize) < bone_transforms.len()
            {
                body.set_kinematic(false);
                body.set_mass(rb.mass.max(0.75));
                body.set_damping(0.12, 0.35);
                body.clear_forces();
                body.set_ragdoll_tag(RAGDOLL_CORE_TAG);
                rb.physics_mode = PhysicsMode::Physics;
                self.dynamic_bone_indices.insert(rb.bone_index as usize);
                self.position_driven_bone_indices
                    .insert(rb.bone_index as usize);
            }
            let group = 1i32 << rb.group.min(15);
            self.world
                .add_rigid_body(body, group, (rb.group_mask | 0xC000) as i32);
        }

        // A few dozen bone constraints benefit from extra solver iterations.
        // Core-body self-collisions are also suppressed by the existing PMX
        // broadphase filter, preventing the immediate overlap explosion.
        self.world.set_solver_iterations(24);
        // The ragdoll-specific broadphase exclusions are implemented in this
        // filter, so they must remain active even if an earlier config disabled
        // the regular MMD kinematic/dynamic filter.
        self.world.set_kinematic_filter(true);

        // Build anatomical joints from the PMX bone hierarchy. Standard PMX
        // models typically have spring constraints for clothing, but none for
        // the kinematic torso, limbs and head.
        let by_bone: HashMap<usize, usize> = original_kinematic
            .iter()
            .map(|&(rb_idx, bone_idx)| (bone_idx, rb_idx))
            .collect();
        // PMX rigs often have unphysical control bones (all-parent, center,
        // groove, waist) above BOTH upper-body and lower-body colliders. If
        // those controls remain upright, meshes weighted to them form tall
        // stretched spikes as the simulated body falls. Follow the primary
        // anatomical root with the topmost unphysical ancestor, then let the
        // regular skeleton hierarchy propagate through the intermediate bones.
        let unattached_roots: Vec<(usize, usize)> = original_kinematic
            .iter()
            .copied()
            .filter(|&(_, bone)| nearest_ragdoll_parent(bone, bone_parents, &by_bone).is_none())
            .collect();
        if let Some(&(anchor_rb, anchor_bone)) = unattached_roots.first() {
            if let Some(root_bone) = highest_ancestor(anchor_bone, bone_parents) {
                if root_bone != anchor_bone
                    && root_bone < bone_transforms.len()
                    && !self.dynamic_bone_indices.contains(&root_bone)
                {
                    let root_offset = bone_transforms[anchor_bone].inverse()
                        * bone_transforms[root_bone];
                    self.ragdoll_root_anchor = Some((root_bone, anchor_rb, root_offset));
                    self.ragdoll_root_bone_indices.insert(root_bone);
                    self.dynamic_bone_indices.insert(root_bone);
                    self.position_driven_bone_indices.insert(root_bone);
                }
            }
        }
        let mut added = 0usize;
        for &(child_rb_idx, child_bone_idx) in &original_kinematic {
            let Some(parent_rb_idx) =
                nearest_ragdoll_parent(child_bone_idx, bone_parents, &by_bone)
            else {
                continue;
            };

            if self.joints.iter().any(|joint| {
                (joint.rigid_body_a_index == parent_rb_idx as i32
                    && joint.rigid_body_b_index == child_rb_idx as i32)
                    || (joint.rigid_body_a_index == child_rb_idx as i32
                        && joint.rigid_body_b_index == parent_rb_idx as i32)
            }) {
                continue;
            }

            let parent = self.rigid_bodies[parent_rb_idx].bullet_body.as_ref().unwrap();
            let child = self.rigid_bodies[child_rb_idx].bullet_body.as_ref().unwrap();
            let pivot = super::inv_z(bone_transforms[child_bone_idx]);
            // Joint frames must be relative to the *current* death-pose
            // bodies, not PMX's bind-pose rigid-body transforms.
            let frame_a = parent.get_transform().inverse() * pivot;
            let frame_b = child.get_transform().inverse() * pivot;
            if let Some(constraint) =
                BulletConstraint::new_6dof_spring(parent, child, frame_a, frame_b, true)
            {
                constraint.set_linear_lower_limit(0.0, 0.0, 0.0);
                constraint.set_linear_upper_limit(0.0, 0.0, 0.0);
                constraint.set_angular_lower_limit(-0.95, -0.95, -0.95);
                constraint.set_angular_upper_limit(0.95, 0.95, 0.95);
                for axis in 0..6 {
                    constraint.set_param(BT_CONSTRAINT_STOP_ERP, 0.475, axis);
                }
                self.world.add_constraint(&constraint, true);
                self.joints.push(MmdJointData {
                    name: format!("ragdoll_{}", child_bone_idx),
                    rigid_body_a_index: parent_rb_idx as i32,
                    rigid_body_b_index: child_rb_idx as i32,
                    constraint: Some(constraint),
                });
                added += 1;
            }
        }

        // Two anatomical roots can share a non-physical pelvis bone. The
        // parent-walk above cannot connect siblings via that missing body,
        // leaving the whole torso as a separate floating physics island.
        // Connect near root siblings at their deepest common PMX ancestor.
        let mut root_bridges = 0usize;
        if let Some(&(anchor_rb, anchor_bone)) = unattached_roots.first() {
            for &(child_rb, child_bone) in unattached_roots.iter().skip(1) {
                let Some(pelvis_bone) = common_ancestor(anchor_bone, child_bone, bone_parents)
                else { continue };
                if pelvis_bone >= bone_transforms.len() { continue; }
                // A shared world-control root alone is not an anatomical
                // joint; do not glue unrelated accessories together.
                if bone_parents.get(pelvis_bone).copied().unwrap_or(-1) < 0 {
                    continue;
                }
                let separation = bone_transforms[anchor_bone].w_axis.truncate()
                    .distance(bone_transforms[child_bone].w_axis.truncate());
                if separation > 6.0 { continue; }
                if self.joints.iter().any(|joint| {
                    (joint.rigid_body_a_index == anchor_rb as i32
                        && joint.rigid_body_b_index == child_rb as i32)
                        || (joint.rigid_body_a_index == child_rb as i32
                            && joint.rigid_body_b_index == anchor_rb as i32)
                }) {
                    continue;
                }

                let parent = self.rigid_bodies[anchor_rb].bullet_body.as_ref().unwrap();
                let child = self.rigid_bodies[child_rb].bullet_body.as_ref().unwrap();
                let pivot = super::inv_z(bone_transforms[pelvis_bone]);
                let frame_a = parent.get_transform().inverse() * pivot;
                let frame_b = child.get_transform().inverse() * pivot;
                if let Some(joint) =
                    BulletConstraint::new_6dof_spring(parent, child, frame_a, frame_b, true)
                {
                    joint.set_linear_lower_limit(0.0, 0.0, 0.0);
                    joint.set_linear_upper_limit(0.0, 0.0, 0.0);
                    joint.set_angular_lower_limit(-0.65, -0.65, -0.65);
                    joint.set_angular_upper_limit(0.65, 0.65, 0.65);
                    for axis in 0..6 {
                        joint.set_param(BT_CONSTRAINT_STOP_ERP, 0.475, axis);
                    }
                    self.world.add_constraint(&joint, true);
                    self.joints.push(MmdJointData {
                        name: format!("ragdoll_pelvis_bridge_{}", pelvis_bone),
                        rigid_body_a_index: anchor_rb as i32,
                        rigid_body_b_index: child_rb as i32,
                        constraint: Some(joint),
                    });
                    root_bridges += 1;
                }
            }
        }

        if let Some(shape) = BulletShape::r#box(32.0, 0.5, 32.0) {
            let info = RigidBodyInfo {
                mass: 0.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                friction: 0.85,
                restitution: 0.0,
                additional_damping: false,
                is_kinematic: false,
                disable_deactivation: true,
                no_contact_response: false,
                initial_transform: Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0)),
            };
            if let Some(floor) = BulletRigidBody::new(&info, &shape) {
                self.world.add_rigid_body(&floor, 0x8000, -1);
                self.ragdoll_floor_body = Some(floor);
                self.ragdoll_floor_shape = Some(shape);
            }
        }

        self.prev_model_position = None;
        log::info!(
            "[MMD ragdoll] {} core bodies dynamic, {} anatomical constraints, {} root bridges, control root={:?}, floor={}",
            original_kinematic.len(), added, root_bridges,
            self.ragdoll_root_anchor.as_ref().map(|&(bone, _, _)| bone),
            self.ragdoll_floor_body.is_some()
        );
        true
    }

    /// Replace the temporary flat floor with real Minecraft collision boxes.
    /// Every input box uses Minecraft units *relative to the entity feet*:
    /// [minX,minY,minZ,maxX,maxY,maxZ]. The local physics scene is mirrored
    /// along Z relative to rendering, then inverse-yawed and divided by the
    /// effective PMX scale (Minecraft blocks per PMX model unit).
    pub fn set_ragdoll_terrain(
        &mut self,
        relative_boxes: &[f32],
        body_yaw_radians: f32,
        model_scale: f32,
    ) -> bool {
        if relative_boxes.len() % 6 != 0
            || relative_boxes.len() > MAX_RAGDOLL_TERRAIN_BOXES * 6
            || !body_yaw_radians.is_finite()
            || !model_scale.is_finite()
            || model_scale <= 0.0001
        {
            return false;
        }

        // Allocate everything first. On native allocation failure, keep the
        // existing emergency floor rather than leaving the ragdoll unsupported.
        let mut new_terrain = Vec::with_capacity(relative_boxes.len() / 6);
        for bounds in relative_boxes.chunks_exact(6) {
            let Some((half_extents, transform)) =
                terrain_box_pose(bounds, body_yaw_radians, model_scale)
            else {
                return false;
            };
            let Some(shape) = BulletShape::r#box(
                half_extents.x, half_extents.y, half_extents.z,
            ) else {
                return false;
            };
            let info = RigidBodyInfo {
                mass: 0.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                friction: 0.85,
                restitution: 0.0,
                additional_damping: false,
                is_kinematic: false,
                disable_deactivation: true,
                no_contact_response: false,
                initial_transform: transform,
            };
            let Some(body) = BulletRigidBody::new(&info, &shape) else {
                return false;
            };
            new_terrain.push(TerrainCollider { body, _shape: shape });
        }

        self.clear_terrain();
        if let Some(floor) = self.ragdoll_floor_body.take() {
            self.world.remove_rigid_body(&floor);
        }
        self.ragdoll_floor_shape = None;
        for terrain in &new_terrain {
            self.world.add_rigid_body(&terrain.body, 0x8000, -1);
        }
        self.ragdoll_terrain = new_terrain;

        log::info!(
            "[MMD ragdoll] Minecraft terrain collision installed: {} boxes",
            self.ragdoll_terrain.len()
        );
        true
    }

    /// Update kinematic collision proxies for nearby live Minecraft actors.
    /// IDs are stable within a client world; all boxes are relative to the
    /// dead entity's original feet, as with the terrain snapshots.
    pub fn set_ragdoll_actors(
        &mut self,
        entity_ids: &[i32],
        relative_boxes: &[f32],
        body_yaw_radians: f32,
        model_scale: f32,
    ) -> bool {
        if entity_ids.len() > MAX_RAGDOLL_ACTORS
            || relative_boxes.len() != entity_ids.len() * 6
            || !body_yaw_radians.is_finite()
            || !model_scale.is_finite()
            || model_scale <= 0.0001
        {
            return false;
        }

        let mut updates = Vec::with_capacity(entity_ids.len());
        let mut unique = HashSet::with_capacity(entity_ids.len());
        for (&id, bounds) in entity_ids.iter().zip(relative_boxes.chunks_exact(6)) {
            if !unique.insert(id) {
                return false;
            }
            let Some((half, pose)) = terrain_box_pose(bounds, body_yaw_radians, model_scale)
            else {
                return false;
            };
            updates.push((id, half, pose));
        }

        let mut index = 0;
        while index < self.ragdoll_actors.len() {
            if unique.contains(&self.ragdoll_actors[index].entity_id) {
                index += 1;
            } else {
                let actor = self.ragdoll_actors.remove(index);
                self.world.remove_rigid_body(&actor.body);
            }
        }

        for (id, half, pose) in updates {
            if let Some(index) = self.ragdoll_actors.iter().position(|actor| actor.entity_id == id) {
                let actor = &self.ragdoll_actors[index];
                if (actor.half_extents - half).abs().max_element() < 0.005 {
                    actor.body.set_transform(pose);
                    continue;
                }
                let old = self.ragdoll_actors.remove(index);
                self.world.remove_rigid_body(&old.body);
            }

            let Some(shape) = BulletShape::r#box(half.x, half.y, half.z) else {
                return false;
            };
            let info = RigidBodyInfo {
                mass: 0.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                friction: 0.8,
                restitution: 0.0,
                additional_damping: false,
                is_kinematic: true,
                disable_deactivation: true,
                no_contact_response: false,
                initial_transform: pose,
            };
            let Some(body) = BulletRigidBody::new(&info, &shape) else {
                return false;
            };
            body.set_ragdoll_tag(RAGDOLL_ACTOR_TAG);
            self.world.add_rigid_body(&body, RAGDOLL_ACTOR_COLLISION_GROUP, -1);
            self.ragdoll_actors.push(ActorCollider {
                entity_id: id,
                half_extents: half,
                body,
                _shape: shape,
            });
        }
        true
    }

    fn clear_terrain(&mut self) {
        for collider in self.ragdoll_terrain.drain(..) {
            self.world.remove_rigid_body(&collider.body);
        }
    }

    fn clear_actors(&mut self) {
        for actor in self.ragdoll_actors.drain(..) {
            self.world.remove_rigid_body(&actor.body);
        }
    }

    /// 重置物理系统
    pub fn reset(&mut self) {
        self.prev_model_position = None;
        for rb_data in &self.rigid_bodies {
            if let Some(ref body) = rb_data.bullet_body {
                body.set_transform(rb_data.initial_transform);
                body.set_linear_velocity(0.0, 0.0, 0.0);
                body.set_angular_velocity(0.0, 0.0, 0.0);
                body.clear_forces();
            }
        }
    }

    /// 设置重力
    pub fn set_gravity(&self, x: f32, y: f32, z: f32) {
        self.world.set_gravity(x, y, z);
    }

    pub fn rigid_body_count(&self) -> usize {
        self.rigid_bodies.len()
    }
    #[cfg(test)]
    pub fn ragdoll_terrain_count(&self) -> usize {
        self.ragdoll_terrain.len()
    }
    #[cfg(test)]
    pub fn ragdoll_actor_count(&self) -> usize {
        self.ragdoll_actors.len()
    }
    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }

    /// 获取动态刚体关联的骨骼变换（复用内部缓冲区，零堆分配）
    ///
    /// 从 Bullet3 读取左手空间变换，通过 inv_z 转回右手空间返回。
    pub fn get_dynamic_bone_transforms(
        &mut self,
        current_bone_transforms: &[Mat4],
    ) -> (
        &[(usize, Mat4, PhysicsBoneUpdate)],
        &HashSet<usize>,
        &HashSet<usize>,
        &HashSet<usize>,
    ) {
        self.dynamic_bone_buf.clear();

        // Synthetic root must be applied before the original rigid-body
        // transforms. The model's control chain has no Bullet collider, but
        // may still carry skin weights ("all parent" is not necessarily empty).
        if let Some((root_bone, anchor_rb, root_offset)) = self.ragdoll_root_anchor {
            if let Some(rb_data) = self.rigid_bodies.get(anchor_rb) {
                if let Some(ref body) = rb_data.bullet_body {
                    let anchor_right = super::inv_z(
                        rb_data.compute_bone_matrix(body.get_transform())
                    );
                    self.dynamic_bone_buf.push((
                        root_bone,
                        anchor_right * root_offset,
                        PhysicsBoneUpdate::RagdollRoot,
                    ));
                }
            }
        }
        for rb_data in &self.rigid_bodies {
            if rb_data.physics_mode == PhysicsMode::FollowBone {
                continue;
            }
            let bone_idx = rb_data.bone_index;
            if bone_idx < 0 || (bone_idx as usize) >= current_bone_transforms.len() {
                continue;
            }
            if let Some(ref body) = rb_data.bullet_body {
                let rb_matrix = body.get_transform();
                let new_bone_left = match rb_data.physics_mode {
                    PhysicsMode::Physics => rb_data.compute_bone_matrix(rb_matrix),
                    PhysicsMode::PhysicsWithBone => {
                        let bone_right = current_bone_transforms[bone_idx as usize];
                        let pos_left = Vec3::new(
                            bone_right.w_axis.x,
                            bone_right.w_axis.y,
                            -bone_right.w_axis.z,
                        );
                        rb_data.compute_bone_matrix_rotation_only(rb_matrix, pos_left)
                    }
                    PhysicsMode::FollowBone => unreachable!(),
                };
                let update_kind = match rb_data.physics_mode {
                    PhysicsMode::Physics => PhysicsBoneUpdate::Dynamic,
                    PhysicsMode::PhysicsWithBone => PhysicsBoneUpdate::RotationOnly,
                    PhysicsMode::FollowBone => unreachable!(),
                };
                self.dynamic_bone_buf.push((
                    bone_idx as usize,
                    super::inv_z(new_bone_left),
                    update_kind,
                ));
            }
        }
        (
            &self.dynamic_bone_buf,
            &self.ragdoll_root_bone_indices,
            &self.position_driven_bone_indices,
            &self.dynamic_bone_indices,
        )
    }

    /// 获取动态骨骼索引集合的引用（构建时已预计算，零分配）
    pub fn get_dynamic_bone_indices(&self) -> &HashSet<usize> {
        &self.dynamic_bone_indices
    }

    /// 获取 C++ 侧存活对象计数（调试用）
    pub fn alloc_stats() -> bullet_ffi::BulletAllocStats {
        bullet_ffi::get_alloc_stats()
    }
}

impl Drop for MMDPhysics {
    fn drop(&mut self) {
        // Bullet3 要求：必须在 destroy 对象前先从世界中移除。
        // Rust 默认按声明顺序 drop 字段（joints → rigid_bodies → world），
        // 如果不先移除，bw_world_destroy 会访问已释放的指针导致崩溃。
        for joint in &self.joints {
            if let Some(ref constraint) = joint.constraint {
                self.world.remove_constraint(constraint);
            }
        }
        for rb in &self.rigid_bodies {
            if let Some(ref body) = rb.bullet_body {
                self.world.remove_rigid_body(body);
            }
        }
        if let Some(ref floor) = self.ragdoll_floor_body {
            self.world.remove_rigid_body(floor);
        }
        self.clear_terrain();
        self.clear_actors();
        // 之后 Rust 自动 drop 各字段（约束/刚体/世界），此时世界已为空，安全释放

        // 泄漏检测日志（仅在当前实例释放后检查全局计数）
        let stats = bullet_ffi::get_alloc_stats();
        if !stats.is_clean() {
            log::warn!(
                "[Bullet3] C++ 侧仍有存活对象: worlds={}, shapes={}, bodies={}, constraints={}, motionStates={}",
                stats.worlds, stats.shapes, stats.rigid_bodies,
                stats.constraints, stats.motion_states
            );
        }
    }
}

/// Translate Minecraft world-space AABB coordinates into Bullet model-local
/// coordinates. The box is yawed about its center to preserve stair/slab/wall
/// geometry rather than taking a loose rotated AABB.
fn terrain_box_pose(bounds: &[f32], yaw: f32, scale: f32) -> Option<(Vec3, Mat4)> {
    if bounds.len() != 6 || !yaw.is_finite() || !scale.is_finite() || scale <= 0.0001
        || bounds.iter().any(|v| !v.is_finite())
    {
        return None;
    }
    let min = Vec3::from_slice(&bounds[0..3]);
    let max = Vec3::from_slice(&bounds[3..6]);
    let half = (max - min) * (0.5 / scale);
    if half.min_element() < 0.0001 || half.max_element() > 1024.0 {
        return None;
    }

    let center = (min + max) * (0.5 / scale);
    let (sin, cos) = yaw.sin_cos();
    let physics_center = Vec3::new(
        cos * center.x + sin * center.z,
        center.y,
        sin * center.x - cos * center.z,
    );
    let transform = Mat4::from_rotation_translation(
        Quat::from_rotation_y(-yaw),
        physics_center,
    );
    Some((half, transform))
}

/// Walk the PMX hierarchy, ignoring intermediary bones with no collision body.
fn nearest_ragdoll_parent(
    child_bone: usize,
    parents: &[i32],
    bodies_by_bone: &HashMap<usize, usize>,
) -> Option<usize> {
    let mut current = child_bone;
    for _ in 0..parents.len() {
        let parent = usize::try_from(*parents.get(current)?).ok()?;
        if parent == current || parent >= parents.len() {
            return None;
        }
        if let Some(&body_idx) = bodies_by_bone.get(&parent) {
            return Some(body_idx);
        }
        current = parent;
    }
    None
}

/// Topmost ancestor of a PMX control chain, including the starting bone.
/// A cycle/invalid parent is treated as bad model data rather than looping.
fn highest_ancestor(start: usize, parents: &[i32]) -> Option<usize> {
    let mut current = start;
    for _ in 0..parents.len() {
        let parent = *parents.get(current)?;
        if parent == -1 {
            return Some(current);
        }
        let next = usize::try_from(parent).ok()?;
        if next == current || next >= parents.len() {
            return None;
        }
        current = next;
    }
    None
}

/// Deepest common control bone used as the missing articulation pivot. A PMX
/// upper-body and lower-body pair can both descend from a collider-free waist.
fn common_ancestor(a: usize, b: usize, parents: &[i32]) -> Option<usize> {
    let mut a_ancestors = HashSet::new();
    let mut current = a;
    for _ in 0..parents.len() {
        a_ancestors.insert(current);
        let next = usize::try_from(*parents.get(current)?).ok();
        match next {
            Some(n) if n < parents.len() && n != current => current = n,
            _ => break,
        }
    }
    let mut current = b;
    for _ in 0..parents.len() {
        if a_ancestors.contains(&current) {
            return Some(current);
        }
        let next = usize::try_from(*parents.get(current)?).ok()?;
        if next >= parents.len() || next == current {
            return None;
        }
        current = next;
    }
    None
}

#[cfg(test)]
mod ragdoll_tests {
    use super::*;

    #[test]
    fn pelvis_without_rigid_body_still_connects_two_anatomical_roots() {
        // 0 all-parent -> 1 center -> 2 waist -> {3 lower, 4 upper}.
        let parents = [-1, 0, 1, 2, 2, 4];
        let by_bone = HashMap::from([(3, 0usize), (4, 1usize), (5, 2usize)]);
        assert_eq!(highest_ancestor(3, &parents), Some(0));
        assert_eq!(common_ancestor(3, 4, &parents), Some(2));
        assert_eq!(nearest_ragdoll_parent(3, &parents, &by_bone), None);
        assert_eq!(nearest_ragdoll_parent(4, &parents, &by_bone), None);
        assert_eq!(nearest_ragdoll_parent(5, &parents, &by_bone), Some(1));
    }

    #[test]
    fn moving_minecraft_actors_keep_a_stable_collision_proxy() {
        let mut physics = MMDPhysics::new().expect("Bullet world");
        let first = [-1.0, 0.0, -0.5, -0.4, 1.8, 0.5];
        assert!(physics.set_ragdoll_actors(&[11], &first, 0.0, 1.0));
        assert_eq!(physics.ragdoll_actor_count(), 1);
        let first_position = physics.ragdoll_actors[0].body.get_position();

        let second = [-0.5, 0.0, -0.5, 0.1, 1.8, 0.5];
        assert!(physics.set_ragdoll_actors(&[11], &second, 0.0, 1.0));
        assert_eq!(physics.ragdoll_actor_count(), 1);
        assert!((physics.ragdoll_actors[0].body.get_position().x - first_position.x - 0.5).abs() < 0.001);

        // Invalid input must preserve the previous collider state.
        assert!(!physics.set_ragdoll_actors(&[11, 11], &[0.0; 12], 0.0, 1.0));
        assert_eq!(physics.ragdoll_actor_count(), 1);
        assert!(physics.set_ragdoll_actors(&[], &[], 0.0, 1.0));
        assert_eq!(physics.ragdoll_actor_count(), 0);
    }

    #[test]
    fn anatomical_bodies_do_not_explode_from_self_collision() {
        let physics = MMDPhysics::new().expect("Bullet world");
        physics.world.set_gravity(0.0, 0.0, 0.0);
        physics.world.set_kinematic_filter(true);
        let shape = BulletShape::sphere(0.5).expect("shape");
        let info = RigidBodyInfo {
            mass: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            friction: 0.5,
            restitution: 0.0,
            additional_damping: false,
            is_kinematic: false,
            disable_deactivation: true,
            no_contact_response: false,
            initial_transform: Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        };
        let a = BulletRigidBody::new(&info, &shape).expect("first body");
        let b = BulletRigidBody::new(&info, &shape).expect("second body");
        a.set_ragdoll_tag(RAGDOLL_CORE_TAG);
        b.set_ragdoll_tag(RAGDOLL_CORE_TAG);
        physics.world.add_rigid_body(&a, 1, -1);
        physics.world.add_rigid_body(&b, 1, -1);
        for _ in 0..15 {
            physics.world.step(1.0 / 60.0, 4, 1.0 / 60.0);
        }
        assert!((a.get_position() - b.get_position()).length() < 0.001,
            "self-overlap should not push apart anatomical bodies");
        physics.world.remove_rigid_body(&a);
        physics.world.remove_rigid_body(&b);
    }

    #[test]
    fn minecraft_boxes_mirror_z_and_rotate_with_entity_yaw() {
        let (half, pose) = terrain_box_pose(&[0.0, 0.0, 1.0, 2.0, 1.0, 3.0], 0.0, 0.5)
            .expect("valid collision box");
        assert!((half - Vec3::new(2.0, 1.0, 2.0)).length() < 0.0001);
        assert!((pose.w_axis.truncate() - Vec3::new(2.0, 1.0, -4.0)).length() < 0.0001);

        let (_, rotated) = terrain_box_pose(
            &[0.0, 0.0, 1.0, 2.0, 1.0, 3.0],
            std::f32::consts::FRAC_PI_2,
            0.5,
        )
        .expect("valid rotated collision box");
        assert!((rotated.w_axis.truncate() - Vec3::new(4.0, 1.0, 2.0)).length() < 0.0001);

        assert!(terrain_box_pose(&[0.0, 0.0, 0.0, 1.0, 0.0, 1.0], 0.0, 1.0).is_none());
        assert!(terrain_box_pose(&[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], f32::NAN, 1.0).is_none());
    }

    #[test]
    fn bullet_ragdoll_terrain_stops_a_falling_rigid_body() {
        let mut physics = MMDPhysics::new().expect("Bullet world");
        assert!(physics.set_ragdoll_terrain(&[-2.0, -1.0, -2.0, 2.0, 0.0, 2.0], 0.0, 1.0));
        assert_eq!(physics.ragdoll_terrain.len(), 1);
        assert!(physics.ragdoll_floor_body.is_none());

        let shape = BulletShape::sphere(0.5).expect("sphere");
        let ball = BulletRigidBody::new(
            &RigidBodyInfo {
                mass: 1.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                friction: 0.8,
                restitution: 0.0,
                additional_damping: false,
                is_kinematic: false,
                disable_deactivation: true,
                no_contact_response: false,
                initial_transform: Mat4::from_translation(Vec3::new(0.0, 4.0, 0.0)),
            },
            &shape,
        )
        .expect("dynamic body");
        physics.world.add_rigid_body(&ball, 1, -1);
        for _ in 0..180 {
            physics.world.step(1.0 / 60.0, 4, 1.0 / 60.0);
        }
        let pos = ball.get_position();
        assert!(pos.is_finite() && pos.y > 0.35 && pos.y < 0.9,
            "ball fell through a Minecraft ground box: {:?}", pos);

        physics.world.remove_rigid_body(&ball);
        drop(ball);
        drop(shape);
        // No colliders means actual open air or void: remove the prototype floor.
        assert!(physics.set_ragdoll_terrain(&[], 0.0, 1.0));
        assert!(physics.ragdoll_terrain.is_empty());
    }

    #[test]
    fn nearest_parent_skips_non_collision_bones() {
        let parents = [-1, 0, 1, 2, 1];
        let by_bone = HashMap::from([(0, 5), (2, 8)]);
        assert_eq!(nearest_ragdoll_parent(3, &parents, &by_bone), Some(8));
        assert_eq!(nearest_ragdoll_parent(4, &parents, &by_bone), Some(5));
        assert_eq!(nearest_ragdoll_parent(0, &parents, &by_bone), None);
    }

    #[test]
    fn malformed_parent_cycle_cannot_hang() {
        let parents = [1, 0, -1];
        assert_eq!(nearest_ragdoll_parent(0, &parents, &HashMap::new()), None);
    }
}
