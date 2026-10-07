import Phaser from 'phaser';
import { fetchHealth } from '../api';

/**
 * Checks the API is reachable, then hands off to the world.
 * No assets are loaded yet; everything is drawn with primitives.
 */
export class BootScene extends Phaser.Scene {
  constructor() {
    super('Boot');
  }

  async create() {
    const label = this.add
      .text(this.scale.width / 2, this.scale.height / 2, 'Connecting to server...', {
        fontFamily: 'monospace',
        fontSize: '18px',
        color: '#c9c9d9',
      })
      .setOrigin(0.5);

    try {
      const health = await fetchHealth();
      label.setText(`Server ${health.version} ${health.status}`);
    } catch (err) {
      label.setText('Server unreachable (running offline)');
      console.warn('API health check failed', err);
    }

    this.time.delayedCall(600, () => this.scene.start('World'));
  }
}
