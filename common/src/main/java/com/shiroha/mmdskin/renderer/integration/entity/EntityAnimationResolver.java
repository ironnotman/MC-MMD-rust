package com.shiroha.mmdskin.renderer.integration.entity;

import com.shiroha.mmdskin.NativeFunc;
import com.shiroha.mmdskin.renderer.integration.DetachedRagdollRegistry;
import com.shiroha.mmdskin.renderer.integration.RagdollTerrainBridge;
import com.shiroha.mmdskin.renderer.runtime.animation.MMDAnimManager;
import com.shiroha.mmdskin.player.runtime.EntityAnimState;
import com.shiroha.mmdskin.renderer.api.RenderParams;
import com.shiroha.mmdskin.renderer.integration.ModelPropertyHelper;
import com.shiroha.mmdskin.renderer.runtime.model.MMDModelManager;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import org.joml.Vector3f;

/**
 * 通用实体动画状态解析器。
 */
public final class EntityAnimationResolver {

    private EntityAnimationResolver() {
    }

    public static void resolve(Entity entity, MMDModelManager.Model model,
                                float entityYaw, float tickDelta, RenderParams params) {

        if (entity instanceof LivingEntity living) {
            params.bodyYaw = Mth.rotLerp(tickDelta, living.yBodyRotO, living.yBodyRot);
        } else {
            params.bodyYaw = entityYaw;
        }
        params.bodyPitch = 0.0f;
        params.translation = new Vector3f(0.0f);

        if (entity instanceof LivingEntity living) {
            if (living.getHealth() <= 0.0f) {
                // Exactly once per entity. Avoid starting the default VMD
                // death pose after the Rust engine has taken over its bones.
                if (model.entityData.stateLayers[0] != EntityAnimState.State.Die) {
                    model.entityData.stateLayers[0] = EntityAnimState.State.Die;
                    boolean ragdollStarted = NativeFunc.GetInst()
                            .StartRagdoll(model.model.getModelHandle());
                    if (ragdollStarted) {
                        RagdollTerrainBridge.install(living, model, params.bodyYaw, tickDelta);
                        DetachedRagdollRegistry.capture(living, model, params.bodyYaw);
                    } else {
                        model.model.changeAnim(
                                MMDAnimManager.GetAnimModel(model.model, "die"), 0);
                    }
                }
                return;
            }
            if (model.entityData.stateLayers[0] == EntityAnimState.State.Die) {
                model.model.resetPhysics();
                model.entityData.invalidateStateLayers();
            }
            if (living.isSleeping()) {
                params.bodyYaw = living.getBedOrientation().toYRot() + 180.0f;
                params.bodyPitch = ModelPropertyHelper.getFloat(model.properties, "sleepingPitch", 0.0f);
                params.translation = ModelPropertyHelper.getVector(model.properties, "sleepingTrans");
                changeAnimOnce(model, EntityAnimState.State.Sleep, 0);
                return;
            }
        }

        boolean hasMovement = entity.getX() - entity.xo != 0.0f
                           || entity.getZ() - entity.zo != 0.0f;

        if (entity.isVehicle() && hasMovement) {
            changeAnimOnce(model, EntityAnimState.State.Driven, 0);
        } else if (entity.isVehicle()) {
            changeAnimOnce(model, EntityAnimState.State.Ridden, 0);
        } else if (entity.isSwimming()) {
            changeAnimOnce(model, EntityAnimState.State.Swim, 0);
        } else if (hasMovement && entity.getVehicle() == null) {
            changeAnimOnce(model, EntityAnimState.State.Walk, 0);
        } else {
            changeAnimOnce(model, EntityAnimState.State.Idle, 0);
        }
    }

    private static void changeAnimOnce(MMDModelManager.Model model,
                                        EntityAnimState.State targetState, int layer) {
        if (model.entityData.stateLayers[layer] != targetState) {
            model.entityData.stateLayers[layer] = targetState;
            String property = EntityAnimState.getPropertyName(targetState);
            model.model.changeAnim(MMDAnimManager.GetAnimModel(model.model, property), layer);
        }
    }
}
