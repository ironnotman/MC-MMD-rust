package com.shiroha.mmdskin.renderer.integration;

import com.shiroha.mmdskin.NativeFunc;
import com.shiroha.mmdskin.renderer.runtime.model.MMDModelManager;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.phys.AABB;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import java.util.Comparator;
import java.util.List;

/**
 * Samples moving Minecraft entity hitboxes for a detached MMD corpse.
 * Bullet runs client-side; these proxies push the corpse but never move the
 * server-side player or mob. No Physics Mod API is linked or required.
 */
public final class RagdollActorBridge {
    private static final Logger LOGGER = LogManager.getLogger();
    private static final int MAX_ACTORS = 16;
    private static final int FLOATS_PER_ACTOR = 6;
    private static final double HORIZONTAL_RADIUS = 3.5;

    private RagdollActorBridge() {
    }

    public static void refresh(ClientLevel level, LivingEntity corpseEntity,
                               MMDModelManager.Model model, double originX, double originY,
                               double originZ, float bodyYawDegrees, float modelScale) {
        if (level == null || model == null || model.model == null
                || !Float.isFinite(bodyYawDegrees) || !Float.isFinite(modelScale)
                || modelScale <= 0.0001f) {
            return;
        }

        // Pull just the living actors close enough to touch the corpse, even
        // after its original entity is removed from the client entity list.
        AABB area = new AABB(
                originX - HORIZONTAL_RADIUS, originY - 3.0, originZ - HORIZONTAL_RADIUS,
                originX + HORIZONTAL_RADIUS, originY + 4.0, originZ + HORIZONTAL_RADIUS);
        List<Entity> nearby = level.getEntities(corpseEntity, area,
                entity -> entity instanceof LivingEntity living
                        && living.isAlive() && !living.isSpectator());
        nearby.sort(Comparator.comparingDouble(entity ->
                entity.distanceToSqr(originX, originY, originZ)));

        int count = Math.min(MAX_ACTORS, nearby.size());
        int[] ids = new int[count];
        float[] boxes = new float[count * FLOATS_PER_ACTOR];
        for (int i = 0; i < count; i++) {
            Entity actor = nearby.get(i);
            AABB box = actor.getBoundingBox();
            ids[i] = actor.getId();
            int at = i * FLOATS_PER_ACTOR;
            boxes[at] = (float) (box.minX - originX);
            boxes[at + 1] = (float) (box.minY - originY);
            boxes[at + 2] = (float) (box.minZ - originZ);
            boxes[at + 3] = (float) (box.maxX - originX);
            boxes[at + 4] = (float) (box.maxY - originY);
            boxes[at + 5] = (float) (box.maxZ - originZ);
        }

        try {
            NativeFunc.GetInst().SetRagdollActors(model.model.getModelHandle(), ids, boxes,
                    bodyYawDegrees * ((float) Math.PI / 180.0f), modelScale);
        } catch (RuntimeException | LinkageError ex) {
            LOGGER.warn("[MMD ragdoll] Could not update live entity collision proxies", ex);
        }
    }
}
