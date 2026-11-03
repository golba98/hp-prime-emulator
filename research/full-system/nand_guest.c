/* Self-authored ARM926 NAND transactions. SPDX-License-Identifier: GPL-2.0-or-later */
typedef unsigned int u32;
typedef unsigned char u8;
#define REG(off) (*(volatile u32 *)(0x4e000000u + (off)))
#define DATA (*(volatile u8 *)0x4e000010u)
#define CMD (*(volatile u8 *)0x4e000008u)
#define ADDR (*(volatile u8 *)0x4e00000cu)
#define BOX ((volatile u32 *)0x30001000u)
static void error(u32 code) { BOX[0] = 0xbad00000u | code; for (;;) {} }
static void wait_ready(void) {
 u32 i;
 for(i=0;i<1000000;i++) if(REG(0x28)&1) return;
 error(1);
}
static void address(u32 page,u32 column) {
 ADDR=column; ADDR=column>>8; ADDR=page; ADDR=page>>8; ADDR=page>>16;
}
static void erase(u32 page) {
 CMD=0x60; ADDR=page; ADDR=page>>8; ADDR=page>>16; CMD=0xd0;
 wait_ready(); CMD=0x70; if(DATA&1)error(2);
}
static void read(u32 page,u32 column) {
 CMD=0; address(page,column); CMD=0x30; wait_ready();
}
static u8 pattern(u32 i) {
 u8 value=(u8)(i*37+11);
#if CASE == 2
 if(i==201)value^=4; /* Break repeated256byte symmetry for nontrivial parity. */
#endif
 return value;
}
static void uart(const char *text) {
 volatile u32 *base=(volatile u32*)0x50000000u;
 base[0]=3; base[1]=0x245;
 while(*text)base[8]=(u8)*text++;
}
#if CASE == 1
void guest_main(void) {
 u32 i,p,code=0;
 BOX[0]=0; REG(4)=0xc1; /* Enable, CE0 selected, softlock clear, ECC locked. */
 CMD=0xff; CMD=0x90; ADDR=0;
 for(i=0;i<5;i++)BOX[2+i]=DATA;
 if(BOX[2]!=0xec || BOX[3]!=0xda)error(3);
 for(p=0;p<3;p++) {
  u32 page=p==0?65:p==1?131071:66;
  erase(page); CMD=0x80; address(page,0);
  for(i=0;i<2112;i++)DATA=pattern(i+p);
  CMD=0x10; wait_ready(); CMD=0x70; if(DATA&1)error(4);
  read(page,0);
  for(i=0;i<2112;i++) {u32 v=DATA; if(v!=pattern(i+p))error(5);code+=v;}
 }
 BOX[8]=code; BOX[9]=3; BOX[10]=2112;
 /* Test-owned bad-block policy: inspect physical marker, skip to next block. */
 CMD=0x80;address(256,2048);DATA=0;CMD=0x10;wait_ready();
 read(256,2048);BOX[11]=DATA;
 if(BOX[11]!=0)error(13);
 p=BOX[11]!=255?320:256;read(p,2048);BOX[12]=p;BOX[13]=DATA;
 if(p!=320 || BOX[13]!=255)error(14);
 BOX[0]=0x600d0019; uart("M19 NAND\n");
}
#elif CASE == 2
static u32 ecc_read(u32 page,u32 stored,int compare) {
 u32 i,code;
 REG(4)=0x61; /* INIT main, unlocked main, locked spare. */
 read(page,0);
 for(i=0;i<512;i++)BOX[32+i%4]=DATA;
 REG(4)=0xc1; code=REG(0x34);
 if(compare) {
  REG(0x14)=(stored&0xff)|((stored&0xff00)<<8);
  REG(0x18)=((stored>>16)&0xff)|((stored>>8)&0xff0000);
  return REG(0x2c);
 }
 return code;
}
void guest_main(void) {
 u32 i,code,e,byte,bit,check=0;
 BOX[0]=0;REG(4)=0xc1;erase(129);CMD=0x80;address(129,0);
 REG(4)=0x61;
 for(i=0;i<512;i++)DATA=pattern(i);
 REG(4)=0xc1; code=REG(0x34); BOX[2]=code;
 CMD=0x10;wait_ready();
 BOX[3]=ecc_read(129,code,1); if(BOX[3]!=0)error(6);
 CMD=0x80;address(129,123);DATA=(u8)(pattern(123)&~2u);CMD=0x10;wait_ready();
 e=ecc_read(129,code,1);BOX[4]=e;
 byte=(e>>7)&2047;bit=(e>>4)&7;
 if((e&3)!=1 || byte!=123 || bit!=1)error(7);
 read(129,0);
 for(i=0;i<512;i++) {u32 v=DATA;if(i==byte)v^=1u<<bit;if(v!=pattern(i))error(8);check+=v;}
 BOX[5]=check;
 BOX[6]=ecc_read(129,ecc_read(129,0,0)^1,1);
 if((BOX[6]&3)!=3)error(9);
 CMD=0x80;address(129,123);DATA=(u8)(pattern(123)&~18u);CMD=0x10;wait_ready();
 BOX[7]=ecc_read(129,code,1);if((BOX[7]&3)!=2)error(10);
 BOX[0]=0x600d0019;uart("M19 ECC\n");
}
#else
/* Synthetic first stage: read stage2 via actual NAND MMIO, CRC32, BX handoff. */
void guest_main(void) {
 u32 i,crc=~0u,n=STAGE_SIZE;
 volatile u8 *destination=(volatile u8*)0x30010000u;
 BOX[0]=0;REG(4)=0xc1;CMD=0xff;
 read(4,0);
 for(i=0;i<n;i++) {
  u32 j;u8 v=DATA;destination[i]=v;crc^=v;
  for(j=0;j<8;j++)crc=(crc>>1)^((0u-(crc&1))&0xedb88320u);
 }
 BOX[2]=~crc;BOX[3]=n;
 if((~crc)!=STAGE_CRC)error(11);
 BOX[4]=0xface0019;
 ((void(*)(void))0x30010000u)();
 error(12);
}
#endif
