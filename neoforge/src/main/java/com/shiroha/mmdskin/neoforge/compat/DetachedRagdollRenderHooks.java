package com.shiroha.mmdskin.neoforge.compat;

import com.shiroha.mmdskin.renderer.integration.DetachedRagdollRegistry;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.neoforge.client.event.RenderLevelStageEvent;

/** Draws native MMD ragdolls independently of the dead Minecraft entity. */
public final class DetachedRagdollRenderHooks {
    @SubscribeEvent
    public void onRenderLevel(RenderLevelStageEvent event) {
        if (event.getStage() == RenderLevelStageEvent.Stage.AFTER_ENTITIES) {
            DetachedRagdollRegistry.render(event.getCamera(), event.getPoseStack());
        }
    }
}
