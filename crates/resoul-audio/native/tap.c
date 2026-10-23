/* SPDX-License-Identifier: GPL-3.0-only
 * Owned LADSPA transport. No FFT, allocation, waiting or I/O in the audio callback. */
#include "ladspa/ladspa.h"
#include <stdatomic.h>
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <math.h>

#define CHANNELS 32
#define FRAMES 1024
#define BLOCKS 8
#define CONTEXTS 16
#define MAX_TOKEN 16777215u
_Static_assert(ATOMIC_INT_LOCK_FREE == 2, "Lock-free indices required");
_Static_assert(sizeof(unsigned int) == 4, "32-bit transport indices required");

#ifdef _WIN32
#define TAP_EXPORT __declspec(dllexport)
#else
#define TAP_EXPORT __attribute__((visibility("default")))
#endif

/* Fixed-width fields form the application ABI, independently of LADSPA's native unsigned long. */
struct Block {
    uint64_t first_frame;
    uint32_t instance;
    uint32_t rate;
    uint32_t channels;
    uint32_t frames;
    float samples[FRAMES * CHANNELS];
};

struct Context {
    atomic_uint write_index;
    atomic_uint read_index;
    atomic_uint dropped;
    atomic_flag producing;
    struct Block blocks[BLOCKS];
};

struct Slot {
    atomic_uint token;
    struct Context *context;
};

struct Instance {
    float *control;
    float *inputs[CHANNELS];
    float *outputs[CHANNELS];
    uint64_t position;
    uint32_t id;
    uint32_t channels;
    uint32_t rate;
};

static struct Slot slots[CONTEXTS];
static atomic_uint serial = 1;
static atomic_uint instance_serial = 1;
static LADSPA_Descriptor descriptors[CHANNELS];
static LADSPA_PortDescriptor ports[CHANNELS][1 + 2 * CHANNELS];
static LADSPA_PortRangeHint hints[1 + 2 * CHANNELS];
static const char *names[1 + 2 * CHANNELS];
static char labels[CHANNELS][24];

/* Never reuse an exhausted serial, including after integer wraparound. Initialization is not realtime. */
static uint32_t take_serial(atomic_uint *counter, uint32_t maximum)
{
    unsigned int value = atomic_load(counter);
    while (value <= maximum) {
        if (atomic_compare_exchange_weak(counter, &value, value + 1))
            return value;
    }
    return 0;
}

/* Allocate only while constructing a native graph; exhaustion fails the optional filter. */
static LADSPA_Handle instantiate(const LADSPA_Descriptor *descriptor, unsigned long rate)
{
    if (!rate || rate > 1536000)
        return NULL;
    unsigned int id = take_serial(&instance_serial, UINT32_MAX - 1);
    if (!id)
        return NULL;
    struct Instance *instance = calloc(1, sizeof(*instance));
    if (instance) {
        instance->id = id;
        instance->rate = (uint32_t)rate;
        instance->channels = (uint32_t)(uintptr_t)descriptor->ImplementationData;
    }
    return instance;
}

/* Borrow host buffers for the next callback; the control identifies an owned application context. */
static void connect_port(LADSPA_Handle handle, unsigned long port, float *data)
{
    struct Instance *instance = handle;
    if (port == 0)
        instance->control = data;
    else if (port <= instance->channels)
        instance->inputs[port - 1] = data;
    else if (port <= 2 * instance->channels)
        instance->outputs[port - 1 - instance->channels] = data;
}

/* Publish bounded interleaved blocks. Overload drops whole remaining input and advances its clock. */
static void run(LADSPA_Handle handle, unsigned long count)
{
    struct Instance *instance = handle;
    for (uint32_t channel = 0; channel < instance->channels; channel++) {
        if (instance->inputs[channel] != instance->outputs[channel])
            memmove(instance->outputs[channel], instance->inputs[channel], count * sizeof(float));
    }
    uint64_t first = instance->position;
    if (count > UINT64_MAX - first)
        return;
    instance->position += count;
    float value = *instance->control;
    if (!isfinite(value) || value < 1 || value > MAX_TOKEN)
        return;
    uint32_t token = (uint32_t)value;
    if ((float)token != value)
        return;
    struct Slot *slot = &slots[token % CONTEXTS];
    if (atomic_load_explicit(&slot->token, memory_order_acquire) != token)
        return;
    struct Context *context = slot->context;
    /* Reconfiguration can overlap producers. Never wait for an old graph's callback. */
    if (atomic_flag_test_and_set_explicit(&context->producing, memory_order_acquire)) {
        atomic_fetch_add_explicit(&context->dropped, 1, memory_order_relaxed);
        return;
    }
    unsigned long offset = 0;
    while (offset < count) {
        unsigned int write = atomic_load_explicit(&context->write_index, memory_order_relaxed);
        unsigned int read = atomic_load_explicit(&context->read_index, memory_order_acquire);
        if (write - read >= BLOCKS || offset >= FRAMES * BLOCKS) {
            atomic_fetch_add_explicit(&context->dropped, 1, memory_order_relaxed);
            break;
        }
        uint32_t frames = count - offset < FRAMES ? (uint32_t)(count - offset) : FRAMES;
        struct Block *block = &context->blocks[write % BLOCKS];
        block->first_frame = first + offset;
        block->instance = instance->id;
        block->rate = instance->rate;
        block->channels = instance->channels;
        block->frames = frames;
        for (uint32_t frame = 0; frame < frames; frame++) {
            for (uint32_t channel = 0; channel < instance->channels; channel++)
                block->samples[frame * instance->channels + channel] = instance->inputs[channel][offset + frame];
        }
        atomic_store_explicit(&context->write_index, write + 1, memory_order_release);
        offset += frames;
    }
    atomic_flag_clear_explicit(&context->producing, memory_order_release);
}

/* The host has stopped callbacks before releasing an instance. Context ownership stays with Rust. */
static void cleanup(LADSPA_Handle handle)
{
    free(handle);
}

/* Called exactly once by the loader before exposing descriptors to any native engine. */
TAP_EXPORT uint32_t resolibre_tap_initialize(void)
{
    names[0] = "Owned context";
    hints[0].HintDescriptor = LADSPA_HINT_INTEGER | LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE;
    hints[0].LowerBound = 1;
    hints[0].UpperBound = MAX_TOKEN;
    for (unsigned int port = 1; port <= 2 * CHANNELS; port++)
        names[port] = "Audio";
    for (unsigned int index = 0; index < CHANNELS; index++) {
        unsigned int channels = index + 1;
        snprintf(labels[index], sizeof(labels[index]), "resolibre_%u", channels);
        ports[index][0] = LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL;
        for (unsigned int channel = 0; channel < channels; channel++) {
            ports[index][1 + channel] = LADSPA_PORT_INPUT | LADSPA_PORT_AUDIO;
            ports[index][1 + channels + channel] = LADSPA_PORT_OUTPUT | LADSPA_PORT_AUDIO;
        }
        descriptors[index] = (LADSPA_Descriptor) {
            .UniqueID = 0, .Label = labels[index], .Name = "ReSolibre owned PCM tap",
            .Maker = "ReSolibre", .Copyright = "GPL-3.0-only",
            .Properties = LADSPA_PROPERTY_HARD_RT_CAPABLE,
            .PortCount = 1 + 2 * channels, .PortDescriptors = ports[index],
            .PortNames = names, .PortRangeHints = hints,
            .ImplementationData = (void *)(uintptr_t)channels,
            .instantiate = instantiate, .connect_port = connect_port, .run = run, .cleanup = cleanup,
        };
    }
    return 1;
}

/* Reserve a unique context. Tokens are exactly representable in LADSPA Float32 controls. */
TAP_EXPORT uint32_t resolibre_tap_open(void)
{
    for (unsigned int attempt = 0; attempt < CONTEXTS; attempt++) {
        unsigned int token = take_serial(&serial, MAX_TOKEN);
        if (!token)
            return 0;
        struct Slot *slot = &slots[token % CONTEXTS];
        unsigned int expected = 0;
        if (!atomic_compare_exchange_strong(&slot->token, &expected, UINT32_MAX))
            continue;
        struct Context *context = calloc(1, sizeof(*context));
        if (!context) {
            atomic_store(&slot->token, 0);
            return 0;
        }
        atomic_flag_clear(&context->producing);
        slot->context = context;
        atomic_store_explicit(&slot->token, token, memory_order_release);
        return token;
    }
    return 0;
}

/* The sole reader owns output; Rust validates fields before interpreting sample memory. */
TAP_EXPORT uint32_t resolibre_tap_read(uint32_t token, struct Block *output)
{
    if (!output || !token || atomic_load(&slots[token % CONTEXTS].token) != token)
        return 0;
    struct Context *context = slots[token % CONTEXTS].context;
    unsigned int read = atomic_load_explicit(&context->read_index, memory_order_relaxed);
    unsigned int write = atomic_load_explicit(&context->write_index, memory_order_acquire);
    if (read == write)
        return 0;
    struct Block *block = &context->blocks[read % BLOCKS];
    size_t length = offsetof(struct Block, samples) + block->frames * block->channels * sizeof(float);
    memcpy(output, block, length);
    atomic_store_explicit(&context->read_index, read + 1, memory_order_release);
    return 1;
}

/* Observe identities already allocated before the application requests graph reconstruction. */
TAP_EXPORT uint32_t resolibre_tap_barrier(void)
{
    return atomic_load(&instance_serial) - 1;
}

/* Close only after every engine using this token is destroyed and its sole reader has stopped. */
TAP_EXPORT void resolibre_tap_close(uint32_t token)
{
    struct Slot *slot = &slots[token % CONTEXTS];
    if (!token || atomic_load(&slot->token) != token)
        return;
    free(slot->context);
    slot->context = NULL;
    atomic_store_explicit(&slot->token, 0, memory_order_release);
}

/* Expose a fixed channel-count descriptor; the adapter chooses from validated source facts. */
TAP_EXPORT const LADSPA_Descriptor *ladspa_descriptor(unsigned long index)
{
    return index < CHANNELS ? &descriptors[index] : NULL;
}
