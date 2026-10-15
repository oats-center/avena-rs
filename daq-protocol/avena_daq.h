/*
 * avena_daq.h - host <-> acquisition board protocol, version 1 (draft).
 *
 * Reference definition of the wire format described in
 * docs/src/reference/acquisition-board.md. The board firmware and the host
 * software (rust-ljm) must both follow this file. Change it only together with
 * the protocol version.
 *
 * Rules that apply to every message:
 *   - Transport is one TCP connection. The board listens on AVD_TCP_PORT; the
 *     host connects. One host at a time.
 *   - All integers are little-endian. Structs are packed: no padding.
 *   - Every message is: avd_header_t, then `payload_len` bytes of payload, then
 *     a uint32 CRC-32 (IEEE 802.3, the zlib/Ethernet polynomial) computed over
 *     the header and the payload.
 *   - Times are nanoseconds since the Unix epoch (UTC, no leap seconds), the
 *     same scale as the rest of the Avena software.
 */
#ifndef AVENA_DAQ_H
#define AVENA_DAQ_H

#include <stdint.h>

#define AVD_PROTOCOL_VERSION 1u
#define AVD_TCP_PORT 7600u
#define AVD_MAGIC 0x51445641u          /* bytes 'A' 'V' 'D' 'Q' on the wire */
#define AVD_MAX_CHANNELS 32u
#define AVD_MAX_PAYLOAD (1u << 20)     /* 1 MiB; larger frames are a protocol error */
#define AVD_INVALID_SAMPLE INT32_MIN   /* sample could not be measured; host stores NaN */

#if defined(__GNUC__) || defined(__clang__)
#define AVD_PACKED __attribute__((packed))
#else
#error "define AVD_PACKED for this compiler"
#endif

/* ---------------------------------------------------------------- header */

typedef struct AVD_PACKED {
    uint32_t magic;        /* AVD_MAGIC */
    uint8_t  version;      /* AVD_PROTOCOL_VERSION */
    uint8_t  type;         /* avd_msg_type_t */
    uint16_t reserved;     /* 0 */
    uint32_t payload_len;  /* bytes after this header, before the CRC */
    uint32_t msg_counter;  /* +1 per message sent in this direction on this connection, from 0 */
} avd_header_t;            /* 16 bytes */

typedef enum {
    AVD_HELLO          = 0x01, /* board -> host, first message after connect   */
    AVD_CONFIGURE      = 0x02, /* host  -> board                               */
    AVD_CONFIGURE_ACK  = 0x03, /* board -> host                                */
    AVD_START          = 0x04, /* host  -> board                               */
    AVD_STOP           = 0x05, /* host  -> board; board answers with STOPPED   */
    AVD_STOPPED        = 0x06, /* board -> host                                */
    AVD_DATA           = 0x10, /* board -> host, while streaming               */
    AVD_STATUS         = 0x11, /* board -> host, once a second, always         */
    AVD_ERROR          = 0x12, /* board -> host                                */
    AVD_TIME_REQUEST   = 0x20, /* host  -> board (only if the board has no PPS) */
    AVD_TIME_REPLY     = 0x21, /* board -> host                                */
    AVD_SET_SHUNT      = 0x30, /* host  -> board, allowed while streaming      */
    AVD_SET_SYNC_OUT   = 0x31, /* host  -> board, optional camera/LED marker   */
} avd_msg_type_t;

/* ----------------------------------------------------------------- HELLO */

enum {                                   /* avd_hello_t.capabilities bits */
    AVD_CAP_SIMULTANEOUS   = 1u << 0,    /* all channels sampled at the same instant */
    AVD_CAP_PPS_INPUT      = 1u << 1,    /* GNSS/PPS time input fitted */
    AVD_CAP_PTP            = 1u << 2,    /* IEEE 1588 on the Ethernet port */
    AVD_CAP_BRIDGE_EXC     = 1u << 3,    /* bridge excitation outputs */
    AVD_CAP_SENSOR_EXC     = 1u << 4,    /* 9-28 V sensor supply outputs */
    AVD_CAP_SHUNT_CAL      = 1u << 5,    /* switchable shunt-calibration resistors */
    AVD_CAP_SYNC_OUT       = 1u << 6,    /* sync output for a camera or LED */
};

typedef struct AVD_PACKED {
    uint16_t protocol_version;     /* AVD_PROTOCOL_VERSION */
    uint16_t hardware_revision;
    uint32_t serial_number;
    uint32_t boot_id;              /* random per power-up; changes after any reset */
    char     model[16];            /* NUL-padded, e.g. "AVENA-DAQ8" */
    char     firmware[24];         /* NUL-padded, e.g. "1.0.3+g1a2b3c4" */
    uint8_t  n_channels;           /* physical analog inputs */
    uint8_t  adc_bits;             /* e.g. 24 */
    uint16_t max_samples_per_frame;
    uint32_t capabilities;         /* AVD_CAP_* */
    uint32_t buffer_seconds_at_max_rate; /* how long the board can hold data if the host stalls */
    uint8_t  n_rates;              /* number of entries that follow */
    uint8_t  reserved[3];
    /* followed by n_rates x avd_rate_t, the only rates CONFIGURE may ask for */
} avd_hello_t;

typedef struct AVD_PACKED {
    uint32_t num;                  /* exact rate = num / den samples per second per channel */
    uint32_t den;                  /* e.g. 2000/1; never a rounded value */
} avd_rate_t;

/* ------------------------------------------------------------- CONFIGURE */

typedef enum {
    AVD_MODE_OFF          = 0,
    AVD_MODE_DIFFERENTIAL = 1,     /* volts, (+) minus (-) */
    AVD_MODE_SINGLE_ENDED = 2,     /* volts, (+) against analog ground */
    AVD_MODE_BRIDGE       = 3,     /* ratiometric: result in V/V of this channel's excitation */
} avd_input_mode_t;

typedef struct AVD_PACKED {
    uint8_t  channel;              /* 0 .. n_channels-1, physical input number */
    uint8_t  mode;                 /* avd_input_mode_t */
    uint16_t range_mv;             /* full-scale input range, +/- millivolts; board picks the
                                      smallest gain that covers it and reports it in the ACK */
    uint16_t excitation_mv;        /* 0 = off; bridge excitation or sensor supply */
    uint8_t  shunt_cal;            /* 0/1, initial state of the shunt resistor */
    uint8_t  reserved[3];
} avd_channel_config_t;            /* 10 bytes */

typedef struct AVD_PACKED {
    avd_rate_t rate;               /* must equal one entry of HELLO's rate list */
    uint16_t   samples_per_frame;  /* samples per channel in each DATA message */
    uint8_t    n_channels;         /* entries that follow; channels not listed are off */
    uint8_t    reserved;
    /* followed by n_channels x avd_channel_config_t, in the order DATA will use */
} avd_configure_t;

/* --------------------------------------------------------- CONFIGURE_ACK */

typedef struct AVD_PACKED {
    uint16_t status;               /* 0 = accepted, otherwise avd_error_code_t; nothing changed */
    uint8_t  n_channels;
    uint8_t  reserved;
    avd_rate_t rate;               /* the rate actually in force (equal to the request) */
    int64_t  group_delay_ns;       /* ADC/filter delay; timestamps already include it */
    /* followed by n_channels x avd_channel_scale_t, same order as CONFIGURE */
} avd_configure_ack_t;

typedef struct AVD_PACKED {
    uint8_t  channel;
    uint8_t  mode;
    uint16_t range_mv;             /* actual range after the gain choice */
    double   scale;                /* value = scale * count + offset; volts, or V/V in bridge mode */
    double   offset;
} avd_channel_scale_t;             /* 20 bytes */

/* ----------------------------------------------------------- START, STOP */

typedef struct AVD_PACKED {
    uint32_t stream_id;            /* chosen by the host; echoed in every DATA message */
} avd_start_t;

typedef struct AVD_PACKED {
    uint32_t stream_id;
    uint64_t samples_sent;         /* per channel, since START */
    uint32_t frames_dropped;       /* frames discarded because the host was too slow */
} avd_stopped_t;

/* ------------------------------------------------------------------ DATA */

enum {                             /* avd_data_t.flags */
    AVD_DATA_GAP_BEFORE  = 1u << 0, /* samples were dropped just before this frame */
    AVD_DATA_TIME_VALID  = 1u << 1, /* first_sample_time_ns comes from a locked clock */
    AVD_DATA_FIRST_FRAME = 1u << 2, /* first frame after START */
};

typedef struct AVD_PACKED {
    uint32_t stream_id;
    uint32_t frame_index;          /* 0, 1, 2 ... since START; +1 per frame even after a gap */
    uint64_t first_sample_index;   /* samples per channel since START; jumps forward over a gap */
    uint64_t first_sample_time_ns; /* when the first sample was taken (centre of its conversion) */
    uint32_t flags;                /* AVD_DATA_* */
    uint32_t overrange_mask;       /* bit n: channel in position n clipped at least once */
    uint32_t shunt_mask;           /* bit n: channel in position n had its shunt switched on */
    uint8_t  n_channels;
    uint8_t  reserved;
    uint16_t n_samples;            /* per channel; normally samples_per_frame */
    /* followed by int32 samples, channel-major (planar):
       all n_samples of channel position 0, then all of position 1, ...
       AVD_INVALID_SAMPLE marks a sample that could not be measured. */
} avd_data_t;                      /* 40 bytes before the samples */

/* ---------------------------------------------------------------- STATUS */

enum {                             /* avd_status_t.clock_state */
    AVD_CLOCK_FREE      = 0,       /* crystal only, never set */
    AVD_CLOCK_HOST_SYNC = 1,       /* set from the host via TIME_REQUEST */
    AVD_CLOCK_PTP       = 2,
    AVD_CLOCK_PPS       = 3,       /* disciplined to GNSS PPS */
    AVD_CLOCK_HOLDOVER  = 4,       /* lost its reference, still running */
};

typedef struct AVD_PACKED {
    uint64_t board_time_ns;
    uint32_t uptime_s;
    uint8_t  clock_state;          /* AVD_CLOCK_* */
    uint8_t  streaming;            /* 0/1 */
    int16_t  board_temp_cdeg;      /* 0.01 degC */
    int32_t  clock_offset_ns;      /* last measured offset to the reference, if any */
    uint16_t supply_in_mv;
    uint16_t analog_rail_mv;
    uint8_t  buffer_fill_pct;      /* host-side send backlog */
    uint8_t  n_channels;
    uint16_t reserved;
    uint32_t frames_dropped;       /* since boot */
    /* followed by n_channels x avd_channel_health_t, one per physical input */
} avd_status_t;

typedef struct AVD_PACKED {
    uint8_t  channel;
    uint8_t  flags;                /* bit0 open input, bit1 excitation fault, bit2 overrange now */
    uint16_t excitation_mv;        /* measured at the connector, 0 if off */
    uint16_t excitation_ma_x10;    /* 0.1 mA */
    uint16_t reserved;
} avd_channel_health_t;            /* 8 bytes */

/* ----------------------------------------------------------------- ERROR */

typedef enum {
    AVD_ERR_NONE             = 0,
    AVD_ERR_BAD_MESSAGE      = 1,  /* CRC, magic, length or version wrong */
    AVD_ERR_UNSUPPORTED_RATE = 2,
    AVD_ERR_BAD_CHANNEL      = 3,
    AVD_ERR_BAD_RANGE        = 4,
    AVD_ERR_EXCITATION       = 5,  /* requested excitation not possible or faulted */
    AVD_ERR_NOT_CONFIGURED   = 6,  /* START before an accepted CONFIGURE */
    AVD_ERR_BUSY             = 7,  /* CONFIGURE while streaming */
    AVD_ERR_ADC              = 8,
    AVD_ERR_INTERNAL         = 9,
} avd_error_code_t;

typedef struct AVD_PACKED {
    uint16_t code;                 /* avd_error_code_t */
    uint8_t  fatal;                /* 1 = streaming stopped */
    uint8_t  reserved;
    char     text[64];             /* NUL-padded, for logs only */
} avd_error_t;

/* ------------------------------------------------------------- TIME SYNC */

typedef struct AVD_PACKED {
    uint64_t host_send_ns;
} avd_time_request_t;

typedef struct AVD_PACKED {
    uint64_t host_send_ns;         /* echoed */
    uint64_t board_receive_ns;
    uint64_t board_send_ns;
} avd_time_reply_t;

/* --------------------------------------------------------- SHUNT, SYNC OUT */

typedef struct AVD_PACKED {
    uint8_t channel;
    uint8_t on;                    /* 0/1 */
} avd_set_shunt_t;

typedef struct AVD_PACKED {
    uint8_t  mode;                 /* 0 off, 1 pulse once, 2 pulse every second on the second */
    uint8_t  reserved;
    uint16_t pulse_ms;
} avd_set_sync_out_t;

/* Sizes are part of the protocol. */
_Static_assert(sizeof(avd_header_t) == 16, "header");
_Static_assert(sizeof(avd_rate_t) == 8, "rate");
_Static_assert(sizeof(avd_hello_t) == 68, "hello");
_Static_assert(sizeof(avd_channel_config_t) == 10, "channel config");
_Static_assert(sizeof(avd_configure_t) == 12, "configure");
_Static_assert(sizeof(avd_configure_ack_t) == 20, "configure ack");
_Static_assert(sizeof(avd_channel_scale_t) == 20, "channel scale");
_Static_assert(sizeof(avd_start_t) == 4, "start");
_Static_assert(sizeof(avd_stopped_t) == 16, "stopped");
_Static_assert(sizeof(avd_data_t) == 40, "data");
_Static_assert(sizeof(avd_status_t) == 32, "status");
_Static_assert(sizeof(avd_channel_health_t) == 8, "health");
_Static_assert(sizeof(avd_error_t) == 68, "error");
_Static_assert(sizeof(avd_time_reply_t) == 24, "time reply");

#endif /* AVENA_DAQ_H */
