#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Raster { width: u32, height: u32 },
    Video,
    Audio,
}

impl MediaKind {
    pub fn prefix_bytes(self) -> u64 {
        match self {
            Self::Image => 0,
            Self::Raster { .. } => 72,
            Self::Video | Self::Audio => 48,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Flow {
    pub kind: MediaKind,
    pub object_id: u64,
    pub payload_bytes: usize,
    pub records: usize,
    pub blocked: bool,
}

impl Flow {
    pub fn body_bytes(&self) -> u64 {
        self.kind
            .prefix_bytes()
            .saturating_add(u64::try_from(self.payload_bytes).unwrap_or(u64::MAX))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Native,
    WebSocketSplit,
    WebSocketCoalesced,
}

impl Delivery {
    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "native-stream",
            Self::WebSocketSplit => "websocket-split",
            Self::WebSocketCoalesced => "websocket-coalesced",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Route {
    pub label: &'static str,
    pub rtt_us: u64,
    pub link_bytes_per_second: u64,
    pub credit_window_bytes: u64,
    pub bulk_media: bool,
    pub legacy_control_penalty_us: u64,
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub id: &'static str,
    pub description: &'static str,
    pub route: Route,
    pub delivery: Delivery,
    pub flows: Vec<Flow>,
    pub queued_bytes_peak: u64,
    pub control_operations: usize,
    pub layout_change: bool,
    pub detach: bool,
}

impl Scenario {
    pub fn record_count(&self) -> usize {
        self.flows
            .iter()
            .filter(|flow| !flow.blocked)
            .map(|flow| flow.records)
            .sum()
    }

    pub fn attempted_record_count(&self) -> usize {
        self.flows.iter().map(|flow| flow.records).sum()
    }

    pub fn media_body_bytes(&self) -> u64 {
        self.flows
            .iter()
            .filter(|flow| !flow.blocked)
            .map(|flow| {
                flow.body_bytes()
                    .saturating_mul(u64::try_from(flow.records).unwrap_or(u64::MAX))
            })
            .sum()
    }

    pub fn maximum_body_bytes(&self) -> u64 {
        self.flows.iter().map(Flow::body_bytes).max().unwrap_or(0)
    }
}

const MIB: usize = 1024 * 1024;
const RASTER_1080P_BYTES: usize = 1920 * 1080 * 4;
const RASTER_4K_BYTES: usize = 3840 * 2160 * 4;

const DIRECT: Route = Route {
    label: "direct-local",
    rtt_us: 200,
    link_bytes_per_second: 5 * 1024 * 1024 * 1024,
    credit_window_bytes: 64 * 1024 * 1024,
    bulk_media: false,
    legacy_control_penalty_us: 0,
};

const SSH_PRIMARY: Route = Route {
    label: "ssh-delay-shim-primary",
    rtt_us: 100_000,
    link_bytes_per_second: 100 * 1000 * 1000 / 8,
    credit_window_bytes: 4 * 1024 * 1024,
    bulk_media: false,
    legacy_control_penalty_us: 0,
};

const SSH_BULK: Route = Route {
    label: "ssh-delay-shim-bulk",
    bulk_media: true,
    ..SSH_PRIMARY
};

const VVMUX: Route = Route {
    label: "vvmux-local",
    rtt_us: 400,
    link_bytes_per_second: 3 * 1024 * 1024 * 1024,
    credit_window_bytes: 32 * 1024 * 1024,
    bulk_media: false,
    legacy_control_penalty_us: 20_000,
};

const BROWSER: Route = Route {
    label: "vvbridge-browser-local",
    rtt_us: 2_000,
    link_bytes_per_second: 1024 * 1024 * 1024,
    credit_window_bytes: 8 * 1024 * 1024,
    bulk_media: false,
    legacy_control_penalty_us: 5_000,
};

fn image(object_id: u64, records: usize) -> Flow {
    Flow {
        kind: MediaKind::Image,
        object_id,
        payload_bytes: 2 * MIB,
        records,
        blocked: false,
    }
}

fn video(object_id: u64, records: usize) -> Flow {
    Flow {
        kind: MediaKind::Video,
        object_id,
        payload_bytes: 256 * 1024,
        records,
        blocked: false,
    }
}

fn audio(object_id: u64, records: usize) -> Flow {
    Flow {
        kind: MediaKind::Audio,
        object_id,
        payload_bytes: 512,
        records,
        blocked: false,
    }
}

pub fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            id: "direct_image",
            description: "Direct Vivi to Vivido encoded-image submission",
            route: DIRECT,
            delivery: Delivery::Native,
            flows: vec![image(1, 8)],
            queued_bytes_peak: 2 * MIB as u64,
            control_operations: 8,
            layout_change: false,
            detach: false,
        },
        Scenario {
            id: "direct_raster_1080p",
            description: "Direct Vivi/Vvrd to Vivido raw 1920x1080 RGBA raster",
            route: DIRECT,
            delivery: Delivery::Native,
            flows: vec![Flow {
                kind: MediaKind::Raster {
                    width: 1920,
                    height: 1080,
                },
                object_id: 2,
                payload_bytes: RASTER_1080P_BYTES,
                records: 3,
                blocked: false,
            }],
            queued_bytes_peak: RASTER_1080P_BYTES as u64 + 72,
            control_operations: 8,
            layout_change: false,
            detach: false,
        },
        Scenario {
            id: "direct_raster_4k",
            description: "Direct Vivi/Vvrd to Vivido raw 3840x2160 RGBA raster",
            route: DIRECT,
            delivery: Delivery::Native,
            flows: vec![Flow {
                kind: MediaKind::Raster {
                    width: 3840,
                    height: 2160,
                },
                object_id: 3,
                payload_bytes: RASTER_4K_BYTES,
                records: 1,
                blocked: false,
            }],
            queued_bytes_peak: RASTER_4K_BYTES as u64 + 72,
            control_operations: 8,
            layout_change: false,
            detach: false,
        },
        Scenario {
            id: "direct_video",
            description: "Direct Vivi to Vivido H.264 access-unit submission",
            route: DIRECT,
            delivery: Delivery::Native,
            flows: vec![video(4, 60)],
            queued_bytes_peak: 2 * (256 * 1024 + 48) as u64,
            control_operations: 16,
            layout_change: false,
            detach: false,
        },
        Scenario {
            id: "direct_linked_av",
            description: "Direct Vivi to Vivido linked video and Opus audio",
            route: DIRECT,
            delivery: Delivery::Native,
            flows: vec![video(5, 60), audio(6, 100)],
            queued_bytes_peak: 2 * (256 * 1024 + 48) as u64 + 8 * (512 + 48) as u64,
            control_operations: 24,
            layout_change: false,
            detach: false,
        },
        Scenario {
            id: "direct_audio_only",
            description: "Direct Vivi to Vivido standalone Opus audio",
            route: DIRECT,
            delivery: Delivery::Native,
            flows: vec![audio(7, 500)],
            queued_bytes_peak: 16 * (512 + 48) as u64,
            control_operations: 16,
            layout_change: false,
            detach: false,
        },
        Scenario {
            id: "ssh_100ms_primary",
            description: "100 ms RTT SSH delay shim with control and media on the primary transport",
            route: SSH_PRIMARY,
            delivery: Delivery::Native,
            flows: vec![video(8, 30), audio(9, 50)],
            queued_bytes_peak: SSH_PRIMARY.credit_window_bytes,
            control_operations: 24,
            layout_change: false,
            detach: false,
        },
        Scenario {
            id: "ssh_100ms_bulk",
            description: "100 ms RTT SSH delay shim with media on VIVID_ENDPOINT_BULK",
            route: SSH_BULK,
            delivery: Delivery::Native,
            flows: vec![video(10, 30), audio(11, 50)],
            queued_bytes_peak: SSH_BULK.credit_window_bytes,
            control_operations: 24,
            layout_change: false,
            detach: false,
        },
        Scenario {
            id: "vvmux_multi_pane_isolation",
            description: "Vvmux visible panes with blocked video credit, live audio, layout, and detach",
            route: VVMUX,
            delivery: Delivery::Native,
            flows: vec![
                video(12, 20),
                video(13, 20),
                Flow {
                    blocked: true,
                    ..video(14, 2)
                },
                audio(15, 100),
            ],
            queued_bytes_peak: 6 * (256 * 1024 + 48) as u64 + 16 * (512 + 48) as u64,
            control_operations: 32,
            layout_change: true,
            detach: true,
        },
        browser(
            "vvbridge_vivido_js_split",
            "Vvbridge to Vivido.js with records split across WebSocket messages",
            Delivery::WebSocketSplit,
            16,
        ),
        browser(
            "vvbridge_vivido_js_coalesced",
            "Vvbridge to Vivido.js with records coalesced in WebSocket messages",
            Delivery::WebSocketCoalesced,
            17,
        ),
        browser(
            "vvbridge_vvweb_split",
            "Vvbridge to Vvweb with records split across WebSocket messages",
            Delivery::WebSocketSplit,
            18,
        ),
        browser(
            "vvbridge_vvweb_coalesced",
            "Vvbridge to Vvweb with records coalesced in WebSocket messages",
            Delivery::WebSocketCoalesced,
            19,
        ),
    ]
}

fn browser(
    id: &'static str,
    description: &'static str,
    delivery: Delivery,
    object_id: u64,
) -> Scenario {
    let body_bytes = 64 * 1024 + 48;
    let buffered_records = if delivery == Delivery::WebSocketCoalesced {
        8
    } else {
        1
    };
    Scenario {
        id,
        description,
        route: BROWSER,
        delivery,
        flows: vec![Flow {
            kind: MediaKind::Video,
            object_id,
            payload_bytes: 64 * 1024,
            records: 200,
            blocked: false,
        }],
        queued_bytes_peak: u64::try_from(body_bytes * buffered_records).unwrap_or(u64::MAX),
        control_operations: 32,
        layout_change: false,
        detach: false,
    }
}
