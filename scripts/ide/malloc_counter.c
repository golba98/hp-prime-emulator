/* Test-only Linux allocation counter for a private GUI process. No application patching.
 * SIGUSR1 appends a monotonic counter snapshot using only async-signal-safe IO.
 */
#define _GNU_SOURCE
#include <stdatomic.h>
#include <stdlib.h>
#include <signal.h>
#include <unistd.h>
#include <fcntl.h>
#include <stdint.h>
extern void *__libc_malloc(size_t);
extern void *__libc_calloc(size_t,size_t);
extern void *__libc_realloc(void *,size_t);
extern void __libc_free(void *);
static _Atomic uint64_t calls,bytes;
static int output=-1;
void *malloc(size_t n){atomic_fetch_add_explicit(&calls,1,memory_order_relaxed);atomic_fetch_add_explicit(&bytes,n,memory_order_relaxed);return __libc_malloc(n);}
void *calloc(size_t a,size_t b){atomic_fetch_add_explicit(&calls,1,memory_order_relaxed);atomic_fetch_add_explicit(&bytes,a*b,memory_order_relaxed);return __libc_calloc(a,b);}
void *realloc(void *p,size_t n){atomic_fetch_add_explicit(&calls,1,memory_order_relaxed);atomic_fetch_add_explicit(&bytes,n,memory_order_relaxed);return __libc_realloc(p,n);}
void free(void *p){__libc_free(p);}
static unsigned number(char *buf,uint64_t n){char digits[32];unsigned count=0;do{digits[count++]=(char)('0'+n%10);n/=10;}while(n);for(unsigned i=0;i<count;i++)buf[i]=digits[count-i-1];return count;}
static void snapshot(int signal_number){(void)signal_number;if(output<0)return;char buf[80];unsigned n=number(buf,atomic_load_explicit(&calls,memory_order_relaxed));buf[n++]=' ';n+=number(buf+n,atomic_load_explicit(&bytes,memory_order_relaxed));buf[n++]='\n';(void)write(output,buf,n);}
__attribute__((constructor)) static void setup(void){const char *path=getenv("PPL_ALLOC_COUNTER");if(path){output=open(path,O_CREAT|O_WRONLY|O_APPEND,0600);struct sigaction action={0};action.sa_handler=snapshot;sigemptyset(&action.sa_mask);sigaction(SIGUSR1,&action,0);}}
