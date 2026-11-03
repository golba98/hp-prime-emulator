/* Original deterministic LCD / matrix / I2C guest control, GPL-2.0-or-later.
 * The touch packet contract is inherited model behavior, not physical parity.
 */
typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;
#define REG(base, off) (*(volatile u32 *)((base) + (off)))
#define LCD 0x4c800000u
#define GPIO 0x56000000u
#define I2C 0x54000000u
#define MB ((volatile u32 *)0x30001000u)
#define FB0 ((volatile u16 *)0x30100000u)
#define FB1 ((volatile u16 *)0x30200000u)
#define STRIDE 336u
static void i2c_write(const u8 *data, u32 count)
{
    REG(I2C, 0) = 0xa0;
    REG(I2C, 4) = 0xd0;
    REG(I2C, 12) = 2;
    REG(I2C, 4) = 0xf0;
    for (u32 i = 0; i < count; i++) {
        REG(I2C, 12) = data[i];
        REG(I2C, 0) = 0xa0;
    }
    REG(I2C, 4) = 0xd0;
}
static void i2c_read(u8 reg, volatile u8 *out, u32 count)
{
    i2c_write(&reg, 1);
    REG(I2C, 4) = 0x90;
    REG(I2C, 12) = 3;
    REG(I2C, 4) = 0xb0;
    for (u32 i = 0; i < count; i++) {
        out[i] = REG(I2C, 12);
        if (i + 1 < count) REG(I2C, 0) = 0xa0;
    }
    REG(I2C, 4) = 0x90;
}
static void scan(void)
{
    REG(GPIO, 0x30) = 0x5555;
    for (u32 i = 0; i < 8; i++) {
        REG(GPIO, 0x34) = 1u << i;
        MB[16+i] = REG(GPIO, 0x64) & 255;
    }
    const u8 address[] = {255, 0, 0};
    i2c_write(address, 3);
    i2c_read(0, (volatile u8 *)&MB[32], 20);
    i2c_read(0x78, (volatile u8 *)&MB[40], 4);
}
void display_main(void)
{
    for (u32 i = 0; i < 64; i++) MB[i] = 0;
    for (u32 y = 0; y < 240; y++) {
        for (u32 x = 0; x < STRIDE; x++) {
            u16 color = ((x >> 3) & 31) << 11 | ((y >> 2) & 63) << 5 | (((x ^ y) >> 3) & 31);
            FB0[y*STRIDE+x] = x < 320 ? color : 0xdead;
            FB1[y*STRIDE+x] = x < 320 ? (u16)~color : 0xbeef;
        }
    }
    REG(LCD, 0x10) = (239u << 11) | 319;
    REG(LCD, 0x28) = 0;
    REG(LCD, 0x2c) = (319u << 11) | 239;
    REG(LCD, 0x30) = 320*240;
    REG(LCD, 0x64) = (u32)FB0;
    REG(LCD, 0x68) = (u32)FB1;
    REG(LCD, 0x7c) = (u32)FB0 + STRIDE*240*2;
    REG(LCD, 0x80) = (u32)FB1 + STRIDE*240*2;
    REG(LCD, 0x94) = (32u << 13) | 640;
    REG(LCD, 0x14) = (1u << 16) | (5u << 2) | 1;
    REG(LCD, 0) = 3;
    MB[3] = 0x19d15a01;
    REG(0x30001100u, 0) = REG(0x30001100u, 0) + 1;
    for (;;) {
        u32 command = MB[1];
        if (command == MB[2]) continue;
        if (command == 2) {
            FB1[23*STRIDE+17] = 0xf81f;
            REG(LCD, 0x14) = (1u << 24) | (1u << 23) | (1u << 16) | (5u << 2) | 1;
            MB[4] = REG(LCD, 0x14);
        } else if (command == 3) {
            REG(LCD, 0) = 0;
            MB[5] = REG(LCD, 0);
        } else if (command == 4) {
            REG(LCD, 0) = 3;
        } else if (command >= 10 && command < 90) {
            scan();
        } else if (command == 99) {
            MB[0] = 0x600d0019;
            REG(0x50000000u, 0x20) = 'D';
            REG(0x50000000u, 0x20) = 'I';
            MB[2] = command;
            return;
        }
        MB[2] = command;
    }
}
