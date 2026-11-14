package com.shiroha.mmdskin.renderer.integration;

import net.minecraft.world.entity.Entity;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import java.util.concurrent.ThreadLocalRandom;

/**
 * Optional Physics Mod 3.0.35 settings bridge. The supplied JAR computes
 * max(animation.speed, MobSetting.getLifetime() + randomFloat *
 * MobSetting.getLifetimeVariance()) at the moment a corpse is created.
 * Reflection keeps Physics Mod completely optional for MmdSkin.
 */
final class PhysicsModLifetimeAdapter {
    private static final Logger LOGGER = LogManager.getLogger();
    private static boolean warned;

    private PhysicsModLifetimeAdapter() {
    }

    record Settings(double seconds, int maxCorpses, boolean fromPhysicsMod) {
    }

    static Settings snapshot(Entity entity) {
        try {
            Class<?> configMobs = Class.forName("net.diebuddies.config.ConfigMobs");
            Object mobSetting = configMobs.getMethod("getMobSetting", Entity.class)
                    .invoke(null, entity);
            Class<?> type = mobSetting.getClass();
            double base = ((Number) type.getMethod("getLifetime").invoke(mobSetting)).doubleValue();
            double variance = ((Number) type.getMethod("getLifetimeVariance")
                    .invoke(mobSetting)).doubleValue();

            Object animation = type.getMethod("getAnimation").invoke(mobSetting);
            double despawnAnimationSeconds = animation == null ? 0.0
                    : ((Number) animation.getClass().getField("speed").get(animation)).doubleValue();

            Class<?> configClient = Class.forName("net.diebuddies.config.ConfigClient");
            int limit = configClient.getField("mobRagdollLimit").getInt(null);
            double seconds = chooseSeconds(base, variance, despawnAnimationSeconds,
                    ThreadLocalRandom.current().nextFloat());
            if (!Double.isFinite(seconds)) {
                throw new IllegalStateException("Invalid Physics Mod corpse lifetime");
            }
            return new Settings(seconds, Math.max(0, limit), true);
        } catch (ReflectiveOperationException | LinkageError | RuntimeException ex) {
            // Physics Mod is optional. No extra dependency is required.
            if (!warned && !(ex instanceof ClassNotFoundException)) {
                LOGGER.warn("[MMD ragdoll] Physics Mod lifetime unavailable; using 4-second fallback", ex);
                warned = true;
            }
            return new Settings(4.0, 16, false);
        }
    }

    static double chooseSeconds(double base, double variance, double despawnSpeed, double roll) {
        if (!Double.isFinite(base) || !Double.isFinite(variance)
                || !Double.isFinite(despawnSpeed) || !Double.isFinite(roll)) {
            return Double.NaN;
        }
        return Math.max(Math.max(0.0, despawnSpeed),
                Math.max(0.0, base) + Math.max(0.0, variance) * Math.max(0.0, Math.min(1.0, roll)));
    }
}
