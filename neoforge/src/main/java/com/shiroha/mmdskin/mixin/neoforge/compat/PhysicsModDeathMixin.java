package com.shiroha.mmdskin.mixin.neoforge.compat;

import com.shiroha.mmdskin.neoforge.compat.PhysicsModRagdollGuard;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.level.Level;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Pseudo;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Physics Mod 3.0.35 calls blockifyEntity on death, constructs a vanilla-model
 * ragdoll and immediately removes the original mob. Keep PMX-rendered entities
 * alive for Minecraft's usual death ticks so the MMD Bullet ragdoll is visible.
 * Vanilla/non-MMD mobs still use Physics Mod's original implementation.
 */
@Pseudo
@Mixin(targets = "net.diebuddies.physics.PhysicsMod", remap = false)
public abstract class PhysicsModDeathMixin {
    @Inject(
            method = "blockifyEntity(Lnet/minecraft/world/level/Level;Lnet/minecraft/world/entity/LivingEntity;)V",
            at = @At("HEAD"),
            cancellable = true,
            remap = false,
            require = 0
    )
    private static void mmdskin$letMmdHandleDeath(Level level, LivingEntity entity, CallbackInfo ci) {
        if (PhysicsModRagdollGuard.shouldRetainForMmdRagdoll(entity)) {
            ci.cancel();
        }
    }
}
