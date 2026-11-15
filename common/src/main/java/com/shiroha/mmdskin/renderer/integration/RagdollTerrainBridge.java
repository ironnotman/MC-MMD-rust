package com.shiroha.mmdskin.renderer.integration;

import com.shiroha.mmdskin.NativeFunc;
import com.shiroha.mmdskin.config.ModelConfigManager;
import com.shiroha.mmdskin.renderer.runtime.model.MMDModelManager;
import net.minecraft.core.BlockPos;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import java.util.Arrays;

/**
 * Takes one bounded snapshot of the nearby Minecraft block collision shapes
 * when a PMX death ragdoll starts. The native Bullet world is model-local, so
 * boxes are sent relative to the interpolated entity feet, not absolute world
 * coordinates (which lose precision far from spawn).
 */
public final class RagdollTerrainBridge {
    private static final Logger LOGGER = LogManager.getLogger();
    private static final int HORIZONTAL_RADIUS = 3;
    private static final int MAX_BOXES = 256;
    private static final int FLOATS_PER_BOX = 6;
    private static final int[] VERTICAL_OFFSETS = {-1, 0, -2, 1, -3, 2, 3, 4};
    private static final float BASE_MODEL_SCALE = 0.09f;

    private RagdollTerrainBridge() {
    }

    /** Called only after StartRagdoll returned true, before the next Bullet step. */
    public static void install(LivingEntity entity, MMDModelManager.Model model,
                               float bodyYawDegrees, float tickDelta) {
        if (entity == null || model == null || model.model == null || !entity.level().isClientSide) {
            return;
        }

        float scale = effectiveModelScale(entity, model);
        if (!Float.isFinite(scale) || scale <= 0.0001f || !Float.isFinite(bodyYawDegrees)) {
            return; // Leave the native emergency floor in place.
        }

        double originX = Mth.lerp(tickDelta, entity.xo, entity.getX());
        double originY = Mth.lerp(tickDelta, entity.yo, entity.getY());
        double originZ = Mth.lerp(tickDelta, entity.zo, entity.getZ());
        float[] boxes = new float[MAX_BOXES * FLOATS_PER_BOX];
        int count = 0;

        try {
            Level level = entity.level();
            CollisionContext context = CollisionContext.of(entity);
            BlockPos.MutableBlockPos cursor = new BlockPos.MutableBlockPos();
            int baseX = Mth.floor(originX);
            int baseY = Mth.floor(originY);
            int baseZ = Mth.floor(originZ);

            // Prioritize nearby blocks and the floor over distant buried rock.
            // This bounds work and JNI payload size even for complex modded blocks.
            outer:
            for (int radius = 0; radius <= HORIZONTAL_RADIUS; radius++) {
                for (int dx = -radius; dx <= radius; dx++) {
                    for (int dz = -radius; dz <= radius; dz++) {
                        if (Math.max(Math.abs(dx), Math.abs(dz)) != radius) {
                            continue;
                        }
                        for (int dy : VERTICAL_OFFSETS) {
                            int blockX = baseX + dx;
                            int blockY = baseY + dy;
                            int blockZ = baseZ + dz;
                            cursor.set(blockX, blockY, blockZ);
                            if (!level.hasChunkAt(cursor)) {
                                continue;
                            }

                            BlockState block = level.getBlockState(cursor);
                            if (block.isAir()) {
                                continue;
                            }
                            VoxelShape shape = block.getCollisionShape(level, cursor, context);
                            if (shape.isEmpty()) {
                                continue;
                            }

                            for (AABB box : shape.toAabbs()) {
                                if (count >= MAX_BOXES) {
                                    break outer;
                                }
                                int i = count * FLOATS_PER_BOX;
                                boxes[i] = (float) (blockX + box.minX - originX);
                                boxes[i + 1] = (float) (blockY + box.minY - originY);
                                boxes[i + 2] = (float) (blockZ + box.minZ - originZ);
                                boxes[i + 3] = (float) (blockX + box.maxX - originX);
                                boxes[i + 4] = (float) (blockY + box.maxY - originY);
                                boxes[i + 5] = (float) (blockZ + box.maxZ - originZ);
                                count++;
                            }
                        }
                    }
                }
            }

            boolean installed = NativeFunc.GetInst().SetRagdollTerrain(
                    model.model.getModelHandle(),
                    Arrays.copyOf(boxes, count * FLOATS_PER_BOX),
                    bodyYawDegrees * ((float) Math.PI / 180.0f),
                    scale);
            if (installed) {
                LOGGER.debug("[MMD ragdoll] terrain snapshot: {} block collision boxes", count);
            } else {
                LOGGER.warn("[MMD ragdoll] terrain snapshot rejected; using fallback floor");
            }
        } catch (RuntimeException | LinkageError ex) {
            LOGGER.warn("[MMD ragdoll] terrain collision sampling failed; using fallback floor", ex);
        }
    }

    /** Shared conversion for Minecraft collision shapes -> PMX local units. */
    static float effectiveModelScale(LivingEntity entity, MMDModelManager.Model model) {
        float size = ModelPropertyHelper.getFloat(model.properties, "size", 1.0f);
        float scale = BASE_MODEL_SCALE
                * ModelConfigManager.getLiveConfig(model.model.getModelName()).modelScale
                * size;
        if (entity.isBaby() && !(entity instanceof Player)) {
            scale *= 0.5f;
        }
        return scale;
    }
}
