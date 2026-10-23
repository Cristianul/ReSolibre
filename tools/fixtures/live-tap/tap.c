/* Test-only LADSPA 1.1 PCM tap. Not linked or loaded by the application. */
#include <ladspa.h>
#include <stdatomic.h>
#include <stdint.h>
#include <string.h>

#define MAX_CHANNELS 16
#define BLOCK_SAMPLES 2048
#define RING_BLOCKS 4

_Static_assert(ATOMIC_INT_LOCK_FREE == 2, "The audio tap requires lock-free indices");

/* Each instance has one producer (LADSPA run) and one external reader. */
struct Block {
    unsigned long length;
    float samples[BLOCK_SAMPLES];
};

struct Channel {
    float *input;
    float *output;
    unsigned long rate;
    atomic_uint write_index;
    atomic_uint read_index;
    atomic_uint dropped;
    struct Block blocks[RING_BLOCKS];
};

static struct Channel channels[MAX_CHANNELS];
static atomic_uint channel_count;

/* Claim fixed storage during graph construction; exhaustion fails initialization. */
static LADSPA_Handle instantiate(const LADSPA_Descriptor *descriptor, unsigned long rate)
{
    (void)descriptor;
    unsigned int index = atomic_fetch_add_explicit(&channel_count, 1, memory_order_relaxed);
    if (index >= MAX_CHANNELS)
        return NULL;

    channels[index].rate = rate;
    return &channels[index];
}

/* Borrow the host's planar buffers only for the following run call. */
static void connect_port(LADSPA_Handle handle, unsigned long port, float *data)
{
    struct Channel *channel = handle;
    if (port == 0)
        channel->input = data;
    else if (port == 1)
        channel->output = data;
}

/* Preserve the filter branch while dropping analysis data whenever its ring is full. */
static void run(LADSPA_Handle handle, unsigned long count)
{
    struct Channel *channel = handle;
    if (channel->output != channel->input)
        memmove(channel->output, channel->input, count * sizeof(float));

    unsigned int write = atomic_load_explicit(&channel->write_index, memory_order_relaxed);
    unsigned int read = atomic_load_explicit(&channel->read_index, memory_order_acquire);
    if (write - read >= RING_BLOCKS) {
        atomic_fetch_add_explicit(&channel->dropped, 1, memory_order_relaxed);
        return;
    }

    struct Block *block = &channel->blocks[write % RING_BLOCKS];
    block->length = count < BLOCK_SAMPLES ? count : BLOCK_SAMPLES;
    memcpy(block->samples, channel->input, block->length * sizeof(float));
    atomic_store_explicit(&channel->write_index, write + 1, memory_order_release);
}

/* Static storage remains valid until the test process exits; never reuse an old graph. */
static void cleanup(LADSPA_Handle handle)
{
    (void)handle;
}

/* Copy one bounded block; only the harness thread calls this consumer function. */
unsigned long resolibre_tap_read(unsigned int index, float *output, unsigned long capacity)
{
    if (index >= MAX_CHANNELS || capacity < BLOCK_SAMPLES)
        return 0;

    struct Channel *channel = &channels[index];
    unsigned int read = atomic_load_explicit(&channel->read_index, memory_order_relaxed);
    unsigned int write = atomic_load_explicit(&channel->write_index, memory_order_acquire);
    if (read == write)
        return 0;

    const struct Block *block = &channel->blocks[read % RING_BLOCKS];
    unsigned long length = block->length;
    memcpy(output, block->samples, length * sizeof(float));
    atomic_store_explicit(&channel->read_index, read + 1, memory_order_release);
    return length;
}

/* Report overflow to prove that an absent consumer does not stop playback. */
unsigned int resolibre_tap_dropped(unsigned int index)
{
    if (index >= MAX_CHANNELS)
        return 0;
    return atomic_load_explicit(&channels[index].dropped, memory_order_relaxed);
}

static const LADSPA_PortDescriptor ports[] = {
    LADSPA_PORT_INPUT | LADSPA_PORT_AUDIO,
    LADSPA_PORT_OUTPUT | LADSPA_PORT_AUDIO,
};
static const char *const names[] = {"Input", "Output"};
static const LADSPA_PortRangeHint hints[2] = {{0}, {0}};
static const LADSPA_Descriptor descriptor = {
    .UniqueID = 0,
    .Label = "resolibre_probe",
    .Properties = LADSPA_PROPERTY_HARD_RT_CAPABLE,
    .Name = "ReSolibre test-only bounded PCM tap",
    .Maker = "ReSolibre",
    .Copyright = "GPL-3.0-only",
    .PortCount = 2,
    .PortDescriptors = ports,
    .PortNames = names,
    .PortRangeHints = hints,
    .instantiate = instantiate,
    .connect_port = connect_port,
    .run = run,
    .cleanup = cleanup,
};

/* Expose a single test descriptor; only explicit absolute paths load this plugin. */
const LADSPA_Descriptor *ladspa_descriptor(unsigned long index)
{
    return index == 0 ? &descriptor : NULL;
}
