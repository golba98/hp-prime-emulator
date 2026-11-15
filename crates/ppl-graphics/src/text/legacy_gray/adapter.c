/* Bounded adapter; upstream scan conversion is unmodified (FTL.TXT).
 * Each call owns its raster and scan-conversion worker. No global raster,
 * font parser, guest memory or callback crosses this boundary. */
#include <stdint.h>
#include <stdlib.h>
#ifdef FT_STATIC_RASTER
#undef FT_STATIC_RASTER
#endif
#define STANDALONE_
#define ft_grays_raster ppl_prime_legacy_raster
#include "ftgrays.c"

int ppl_prime_gray(const int64_t *xy, size_t count, const int8_t *tags,
                   const int16_t *contours, size_t contour_count, int32_t flags,
                   int64_t xmin, int64_t ymin, uint32_t width, uint32_t height,
                   unsigned char *pixels, size_t pixel_count) {
    if (!xy || !tags || !contours || !pixels || !count || count > 32767 ||
        !contour_count || contour_count > count || width > 1024 || height > 1024 ||
        pixel_count != (size_t)width * height || xmin < -16384 || xmin > 16384 ||
        ymin < -16384 || ymin > 16384) return -100;
    for (size_t i=0; i<contour_count; ++i) {
        if (contours[i] < 0 || (size_t)contours[i] >= count ||
            (i && contours[i] <= contours[i-1])) return -101;
    }
    if ((size_t)contours[contour_count-1] != count-1) return -101;
    FT_Vector *points = (FT_Vector*)malloc(count * sizeof(FT_Vector));
    if (!points) return -102;
    for (size_t i=0; i<count; ++i) {
        int64_t x=xy[2*i], y=xy[2*i+1];
        if (x < -1048576 || x > 1048576 || y < -1048576 || y > 1048576 ||
            (tags[i]&3) == 3) { free(points); return -103; }
        points[i].x=(long)(x-xmin*64); points[i].y=(long)(y-ymin*64);
    }
    FT_Outline outline = {0};
    outline.n_points=(short)count; outline.n_contours=(short)contour_count;
    outline.points=points; outline.tags=(char*)tags;
    outline.contours=(short*)contours; outline.flags=flags;
    FT_Bitmap bitmap = {0};
    bitmap.width=width; bitmap.rows=height; bitmap.pitch=(int)width;
    bitmap.pixel_mode=FT_PIXEL_MODE_GRAY; bitmap.num_grays=256; bitmap.buffer=pixels;
    FT_Raster_Params params = {0};
    params.target=&bitmap; params.source=&outline; params.flags=FT_RASTER_FLAG_AA;
    gray_TRaster raster={0};
    int result=gray_raster_render((FT_Raster)&raster,&params);
    free(points);
    return result;
}
