import Phaser from 'phaser';

const TILE = 32;
const SPEED = 160;

/**
 * Placeholder world: a grid, a player square, and WASD / arrow movement.
 * Replace with tilemaps and server-authoritative movement later.
 */
export class WorldScene extends Phaser.Scene {
  private player!: Phaser.Physics.Arcade.Image;
  private cursors!: Phaser.Types.Input.Keyboard.CursorKeys;
  private wasd!: Record<'W' | 'A' | 'S' | 'D', Phaser.Input.Keyboard.Key>;

  constructor() {
    super('World');
  }

  create() {
    this.drawGrid();

    // Player is a plain 24x24 texture generated at runtime.
    const g = this.add.graphics();
    g.fillStyle(0x7fd1ff, 1).fillRect(0, 0, 24, 24);
    g.generateTexture('player', 24, 24);
    g.destroy();

    this.player = this.physics.add
      .image(this.scale.width / 2, this.scale.height / 2, 'player')
      .setCollideWorldBounds(true);

    const kb = this.input.keyboard!;
    this.cursors = kb.createCursorKeys();
    this.wasd = kb.addKeys('W,A,S,D') as WorldScene['wasd'];

    this.add.text(8, 8, 'Nightfall  |  move: WASD / arrows', {
      fontFamily: 'monospace',
      fontSize: '14px',
      color: '#8a8aa3',
    });
  }

  update() {
    const left = this.cursors.left.isDown || this.wasd.A.isDown;
    const right = this.cursors.right.isDown || this.wasd.D.isDown;
    const up = this.cursors.up.isDown || this.wasd.W.isDown;
    const down = this.cursors.down.isDown || this.wasd.S.isDown;

    const vx = (right ? 1 : 0) - (left ? 1 : 0);
    const vy = (down ? 1 : 0) - (up ? 1 : 0);
    this.player.setVelocity(vx * SPEED, vy * SPEED);
    if (vx && vy) this.player.body!.velocity.normalize().scale(SPEED);
  }

  private drawGrid() {
    const g = this.add.graphics();
    g.lineStyle(1, 0x1d1d2b, 1);
    for (let x = 0; x <= this.scale.width; x += TILE) g.lineBetween(x, 0, x, this.scale.height);
    for (let y = 0; y <= this.scale.height; y += TILE) g.lineBetween(0, y, this.scale.width, y);
  }
}
