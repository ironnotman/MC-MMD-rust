package com.shiroha.mmdskin.neoforge.compat;

import com.shiroha.mmdskin.asset.catalog.ModelInfo;
import com.shiroha.mmdskin.config.ConfigManager;
import com.shiroha.mmdskin.config.UIConstants;
import com.shiroha.mmdskin.neoforge.YsmCompat;
import com.shiroha.mmdskin.renderer.integration.entity.MobReplacementService;
import com.shiroha.mmdskin.renderer.integration.player.PlayerPerformanceGate;
import com.shiroha.mmdskin.ui.network.PlayerModelSyncManager;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.world.entity.LivingEntity;

/**
 * Prevents Physics Mod from removing an entity that MmdSkin needs to render
 * its own PMX/PMD ragdoll. Only invoked from Physics Mod's client death path.
 * No classes from Physics Mod are linked at runtime.
 */
public final class PhysicsModRagdollGuard {
    private PhysicsModRagdollGuard() {
    }

    public static boolean shouldRetainForMmdRagdoll(LivingEntity entity) {
        if (entity == null || !entity.level().isClientSide || !ConfigManager.isPhysicsEnabled()) {
            return false;
        }

        if (entity instanceof AbstractClientPlayer player) {
            if (player.isSpectator() || YsmCompat.isYsmActive(player)
                    || !PlayerPerformanceGate.allowsMmd(player)) {
                return false;
            }

            Minecraft minecraft = Minecraft.getInstance();
            boolean local = minecraft.player != null && minecraft.player.getUUID().equals(player.getUUID());
            String selected = PlayerModelSyncManager.getPlayerModel(
                    player.getUUID(), player.getName().getString(), local);
            return hasSupportedMmdModel(selected);
        }

        String replacement = MobReplacementService.getReplacementModelName(entity);
        return replacement != null
                && hasSupportedMmdModel(replacement)
                && PlayerPerformanceGate.allowsMobReplacement(entity);
    }

    private static boolean hasSupportedMmdModel(String name) {
        if (name == null || name.isBlank() || UIConstants.DEFAULT_MODEL_NAME.equals(name)) {
            return false;
        }
        ModelInfo info = ModelInfo.findByFolderName(name);
        // The native ragdoll handles PMX/PMD rigid bodies, not VRM skeletons.
        return info != null && !info.isVRM();
    }
}
