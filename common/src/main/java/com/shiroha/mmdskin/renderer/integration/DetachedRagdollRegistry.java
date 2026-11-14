package com.shiroha.mmdskin.renderer.integration;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.PoseStack;
import com.shiroha.mmdskin.renderer.compat.IrisCompat;
import com.shiroha.mmdskin.renderer.runtime.model.MMDModelManager;
import com.shiroha.mmdskin.renderer.runtime.model.AbstractMMDModel;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.phys.Vec3;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import java.util.ArrayList;
import java.util.List;

/**
 * Client-only MMD corpse snapshots; no server-side ghost entities and no
 * dependencies on Physics Mod classes. NeoForge enables this manager and
 * drives its tick and post-entity world-render hooks.
 */
public final class DetachedRagdollRegistry {
    private static final Logger LOGGER = LogManager.getLogger();
    private static final List<Corpse> CORPSES = new ArrayList<>();
    private static ClientLevel world;
    private static boolean enabled;

    private DetachedRagdollRegistry() {
    }

    public static void enableForNeoForgeClient() {
        enabled = true;
    }

    public static boolean isCaptured(LivingEntity entity) {
        if (!enabled || entity == null) return false;
        for (Corpse corpse : CORPSES) {
            if (corpse.original == entity) return true;
        }
        return false;
    }

    /** Run after native StartRagdoll succeeded but before its initial render. */
    public static void capture(LivingEntity entity, MMDModelManager.Model model, float yawDegrees) {
        if (!enabled || entity == null || model == null || model.model == null
                || !(model.model instanceof AbstractMMDModel)
                || !(entity.level() instanceof ClientLevel level)
                || isCaptured(entity)) {
            return;
        }

        PhysicsModLifetimeAdapter.Settings settings = PhysicsModLifetimeAdapter.snapshot(entity);
        if (settings.maxCorpses() == 0 || settings.seconds() <= 0.0) return;
        if (world != level) clear();

        // Move the existing Bullet/GL state, not a newly loaded model, into
        // a private cache key so an eventual player respawn cannot reset it.
        if (!MMDModelManager.detachForRagdoll(model)) {
            LOGGER.warn("[MMD ragdoll] Could not detach dead model from live cache");
            return;
        }

        model.loadModelProperties(false);
        float size = ModelPropertyHelper.getFloat(model.properties, "size", 1.0f);
        long born = level.getGameTime();
        long durationTicks = Math.max(1L, (long) Math.ceil(settings.seconds() * 20.0));
        Corpse corpse = new Corpse(entity, model, level, born + durationTicks,
                entity.getX(), entity.getY(), entity.getZ(),
                yawDegrees, size, entity.isBaby() && !(entity instanceof Player));
        world = level;
        CORPSES.add(corpse);
        while (CORPSES.size() > settings.maxCorpses()) {
            removeAndRelease(0);
        }
        LOGGER.info("[MMD ragdoll] Corpse retained for {} ticks (Physics Mod setting={})",
                durationTicks, settings.fromPhysicsMod());
    }

    /** Called from client post-tick so offscreen corpses continue simulating. */
    public static void tick() {
        if (!enabled) return;
        ClientLevel level = Minecraft.getInstance().level;
        if (level == null || (world != null && world != level)) {
            clear();
            return;
        }
        if (CORPSES.isEmpty()) return;

        long now = level.getGameTime();
        for (int i = CORPSES.size() - 1; i >= 0; --i) {
            Corpse corpse = CORPSES.get(i);
            if (now >= corpse.expiresAt || corpse.original.getHealth() > 0.0f) {
                removeAndRelease(i);
                continue;
            }
            if (!MMDModelManager.touchDetachedRagdoll(corpse.model)) {
                CORPSES.remove(i); // a model reload already disposed this model
                continue;
            }
            ((AbstractMMDModel) corpse.model.model).tickDetachedRagdoll();
        }
    }

    /** Draws from the regular NeoForge world stage, even after entity removal. */
    public static void render(Camera camera, PoseStack worldPoseStack) {
        Minecraft minecraft = Minecraft.getInstance();
        if (!enabled || camera == null || minecraft.level == null || minecraft.level != world
                || CORPSES.isEmpty() || IrisCompat.isRenderingShadows()) {
            return;
        }
        Vec3 cameraPosition = camera.getPosition();
        for (Corpse corpse : CORPSES) {
            // The original entity renderer paints the very first death frame.
            if (corpse.firstRender) {
                corpse.firstRender = false;
                continue;
            }
            if (corpse.world != minecraft.level) continue;
            Vec3 origin = new Vec3(corpse.x, corpse.y, corpse.z);
            if (origin.distanceToSqr(cameraPosition) > 128.0 * 128.0) continue;
            BlockPos blockPos = BlockPos.containing(corpse.x, corpse.y, corpse.z);
            if (!minecraft.level.hasChunkAt(blockPos)) continue;
            if (!MMDModelManager.touchDetachedRagdoll(corpse.model)) continue;

            int packedLight = LightTexture.pack(
                    minecraft.level.getBrightness(LightLayer.BLOCK, blockPos),
                    minecraft.level.getBrightness(LightLayer.SKY, blockPos));
            PoseStack poseStack = worldPoseStack == null ? new PoseStack() : worldPoseStack;
            poseStack.pushPose();
            try {
                poseStack.translate(corpse.x - cameraPosition.x,
                        corpse.y - cameraPosition.y, corpse.z - cameraPosition.z);
                if (corpse.baby) poseStack.scale(0.5f, 0.5f, 0.5f);
                poseStack.scale(corpse.scale, corpse.scale, corpse.scale);
                RenderSystem.setShader(GameRenderer::getRendertypeEntityTranslucentShader);
                ((AbstractMMDModel) corpse.model.model).renderDetachedRagdoll(
                        corpse.original, corpse.yaw, poseStack, packedLight);
            } catch (RuntimeException | LinkageError ex) {
                LOGGER.warn("[MMD ragdoll] Detached corpse rendering failed", ex);
            } finally {
                poseStack.popPose();
            }
        }
    }

    public static void clear() {
        for (Corpse corpse : CORPSES) {
            MMDModelManager.releaseDetachedRagdoll(corpse.model);
        }
        CORPSES.clear();
        world = null;
    }

    private static void removeAndRelease(int index) {
        Corpse corpse = CORPSES.remove(index);
        MMDModelManager.releaseDetachedRagdoll(corpse.model);
    }

    private static final class Corpse {
        final LivingEntity original;
        final MMDModelManager.Model model;
        final ClientLevel world;
        final long expiresAt;
        final double x, y, z;
        final float yaw, scale;
        final boolean baby;
        boolean firstRender = true;

        Corpse(LivingEntity original, MMDModelManager.Model model, ClientLevel world,
               long expiresAt, double x, double y, double z,
               float yaw, float scale, boolean baby) {
            this.original = original;
            this.model = model;
            this.world = world;
            this.expiresAt = expiresAt;
            this.x = x;
            this.y = y;
            this.z = z;
            this.yaw = yaw;
            this.scale = scale;
            this.baby = baby;
        }
    }
}
