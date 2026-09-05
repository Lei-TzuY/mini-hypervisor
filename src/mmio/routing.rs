use std::fmt;

pub const LEGACY_PIC_MASTER_VECTOR_BASE: u8 = 0x40;
pub const LEGACY_PIC_MASTER_GSI_COUNT: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyPicMmioInterruptRoute {
    device_address: u64,
    gsi: u32,
    vector: u8,
}

impl LegacyPicMmioInterruptRoute {
    pub fn new(
        device_address: u64,
        gsi: u32,
    ) -> Result<Self, LegacyPicMmioInterruptRoutingError> {
        if gsi >= LEGACY_PIC_MASTER_GSI_COUNT {
            return Err(LegacyPicMmioInterruptRoutingError::GsiOutsideMasterPic {
                gsi,
            });
        }
        let offset = u8::try_from(gsi).expect("master PIC GSI is bounded below eight");
        let vector = LEGACY_PIC_MASTER_VECTOR_BASE
            .checked_add(offset)
            .expect("legacy PIC master vector range ends below 0x48");
        Ok(Self {
            device_address,
            gsi,
            vector,
        })
    }

    #[must_use]
    pub const fn device_address(self) -> u64 {
        self.device_address
    }

    #[must_use]
    pub const fn gsi(self) -> u32 {
        self.gsi
    }

    #[must_use]
    pub const fn vector(self) -> u8 {
        self.vector
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPicMmioInterruptRoutes {
    routes: Vec<LegacyPicMmioInterruptRoute>,
}

impl LegacyPicMmioInterruptRoutes {
    pub fn new(
        routes: Vec<LegacyPicMmioInterruptRoute>,
    ) -> Result<Self, LegacyPicMmioInterruptRoutingError> {
        if routes.is_empty() {
            return Err(LegacyPicMmioInterruptRoutingError::NoRoutes);
        }
        for (index, route) in routes.iter().copied().enumerate() {
            for existing in &routes[..index] {
                if existing.device_address() == route.device_address() {
                    return Err(
                        LegacyPicMmioInterruptRoutingError::DuplicateDeviceAddress {
                            device_address: route.device_address(),
                        },
                    );
                }
                if existing.gsi() == route.gsi() {
                    return Err(LegacyPicMmioInterruptRoutingError::DuplicateGsi {
                        gsi: route.gsi(),
                    });
                }
            }
        }
        Ok(Self { routes })
    }

    #[must_use]
    pub fn routes(&self) -> &[LegacyPicMmioInterruptRoute] {
        &self.routes
    }

    #[must_use]
    pub fn route_for_device(&self, device_address: u64) -> Option<LegacyPicMmioInterruptRoute> {
        self.routes
            .iter()
            .copied()
            .find(|route| route.device_address() == device_address)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyPicMmioInterruptRoutingError {
    NoRoutes,
    GsiOutsideMasterPic { gsi: u32 },
    DuplicateDeviceAddress { device_address: u64 },
    DuplicateGsi { gsi: u32 },
}

impl fmt::Display for LegacyPicMmioInterruptRoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRoutes => write!(f, "legacy-PIC MMIO interrupt routing requires at least one route"),
            Self::GsiOutsideMasterPic { gsi } => write!(
                f,
                "GSI {gsi} is outside the bounded legacy master-PIC range 0..{LEGACY_PIC_MASTER_GSI_COUNT}"
            ),
            Self::DuplicateDeviceAddress { device_address } => write!(
                f,
                "MMIO interrupt source {device_address:#x} has more than one legacy-PIC route"
            ),
            Self::DuplicateGsi { gsi } => write!(
                f,
                "legacy-PIC GSI {gsi} is assigned to more than one MMIO interrupt source"
            ),
        }
    }
}

impl std::error::Error for LegacyPicMmioInterruptRoutingError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_pic_routes_derive_exact_vectors() {
        let first = LegacyPicMmioInterruptRoute::new(0x1000_0000, 0).unwrap();
        let second = LegacyPicMmioInterruptRoute::new(0x1000_1000, 1).unwrap();
        assert_eq!(first.vector(), 0x40);
        assert_eq!(second.vector(), 0x41);

        let routes = LegacyPicMmioInterruptRoutes::new(vec![first, second]).unwrap();
        assert_eq!(routes.routes(), &[first, second]);
        assert_eq!(routes.route_for_device(0x1000_0000), Some(first));
        assert_eq!(routes.route_for_device(0x1000_1000), Some(second));
        assert_eq!(routes.route_for_device(0x1000_2000), None);
    }

    #[test]
    fn rejects_master_pic_overflow_and_ambiguous_route_sets() {
        assert_eq!(
            LegacyPicMmioInterruptRoute::new(0x1000_0000, 8),
            Err(LegacyPicMmioInterruptRoutingError::GsiOutsideMasterPic { gsi: 8 })
        );
        assert_eq!(
            LegacyPicMmioInterruptRoutes::new(vec![]),
            Err(LegacyPicMmioInterruptRoutingError::NoRoutes)
        );

        let first = LegacyPicMmioInterruptRoute::new(0x1000_0000, 0).unwrap();
        let same_device = LegacyPicMmioInterruptRoute::new(0x1000_0000, 1).unwrap();
        assert_eq!(
            LegacyPicMmioInterruptRoutes::new(vec![first, same_device]),
            Err(LegacyPicMmioInterruptRoutingError::DuplicateDeviceAddress {
                device_address: 0x1000_0000
            })
        );

        let same_gsi = LegacyPicMmioInterruptRoute::new(0x1000_1000, 0).unwrap();
        assert_eq!(
            LegacyPicMmioInterruptRoutes::new(vec![first, same_gsi]),
            Err(LegacyPicMmioInterruptRoutingError::DuplicateGsi { gsi: 0 })
        );
    }
}
