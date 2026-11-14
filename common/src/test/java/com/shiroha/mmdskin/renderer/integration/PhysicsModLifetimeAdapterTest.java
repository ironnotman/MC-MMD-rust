package com.shiroha.mmdskin.renderer.integration;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class PhysicsModLifetimeAdapterTest {

    @Test
    void usesPhysicsModLifetimeAndRandomVariance() {
        assertEquals(4.0, PhysicsModLifetimeAdapter.chooseSeconds(4.0, 3.0, 0.5, 0.0));
        assertEquals(5.5, PhysicsModLifetimeAdapter.chooseSeconds(4.0, 3.0, 0.5, 0.5));
        assertEquals(7.0, PhysicsModLifetimeAdapter.chooseSeconds(4.0, 3.0, 0.5, 1.0));
    }

    @Test
    void neverExpiresBeforeDespawnAnimationCanFinish() {
        assertEquals(8.0, PhysicsModLifetimeAdapter.chooseSeconds(4.0, 3.0, 8.0, 0.2));
    }

    @Test
    void rejectsInvalidDurations() {
        assertTrue(Double.isNaN(PhysicsModLifetimeAdapter.chooseSeconds(
                Double.NaN, 3.0, 0.5, 0.5)));
        assertTrue(Double.isNaN(PhysicsModLifetimeAdapter.chooseSeconds(
                4.0, 3.0, Double.POSITIVE_INFINITY, 0.5)));
    }
}
