#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedVirtioBlkCheckpoint {
    guest: BoundedVcpuPageSetCheckpoint,
    bar0: u64,
    device: crate::portio::pci::virtio_blk::VirtioBlkDevice,
}

impl BoundedVirtioBlkCheckpoint {
    pub fn capture(
        vcpu: &Vcpu,
        msr_policy: &GuestMsrAccessPolicy,
        memory: &GuestMemory,
        mmio: &crate::mmio::MmioBus,
        bar0: u64,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let guest = BoundedVcpuPageSetCheckpoint::capture(
            vcpu,
            msr_policy,
            memory,
            page_addresses,
        )?;
        let device = mmio
            .capture_virtio_blk_checkpoint_at(bar0)?
            .ok_or_else(|| {
                page_set_error(
                    "bounded virtio-blk checkpoint capture",
                    format!("no virtio-blk device is registered at BAR {bar0:#x}"),
                )
            })?;
        Ok(Self {
            guest,
            bar0,
            device,
        })
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        self.guest.pages()
    }

    #[must_use]
    pub const fn vcpu(&self) -> &VcpuStateSnapshot {
        self.guest.vcpu()
    }

    #[must_use]
    pub const fn bar0(&self) -> u64 {
        self.bar0
    }

    #[must_use]
    pub const fn device(&self) -> &crate::portio::pci::virtio_blk::VirtioBlkDevice {
        &self.device
    }

    pub fn verify(
        &self,
        vcpu: &Vcpu,
        memory: &GuestMemory,
        mmio: &crate::mmio::MmioBus,
    ) -> Result<BoundedVirtioBlkCheckpointComparison, Error> {
        let guest = self.guest.verify(vcpu, memory)?;
        let device = mmio
            .verify_virtio_blk_checkpoint_at(self.bar0, &self.device)?
            .unwrap_or(false);
        Ok(BoundedVirtioBlkCheckpointComparison { guest, device })
    }

    pub fn restore_and_verify(
        &self,
        vcpu: &Vcpu,
        memory: &mut GuestMemory,
        mmio: &mut crate::mmio::MmioBus,
    ) -> Result<BoundedVirtioBlkCheckpointComparison, Error> {
        let guest = self.guest.restore_and_verify(vcpu, memory)?;
        if !guest.is_exact_match() {
            return Err(page_set_error(
                "bounded virtio-blk checkpoint guest restore verification",
                "page/VCPU state was not exact after restore; device state was not mutated",
            ));
        }

        mmio.restore_virtio_blk_checkpoint_at(self.bar0, &self.device)?
            .ok_or_else(|| {
                page_set_error(
                    "bounded virtio-blk checkpoint device restore",
                    format!("virtio-blk device at BAR {:#x} disappeared", self.bar0),
                )
            })?;

        self.verify(vcpu, memory, mmio)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedVirtioBlkCheckpointComparison {
    guest: BoundedPageSetCheckpointComparison,
    device: bool,
}

impl BoundedVirtioBlkCheckpointComparison {
    #[must_use]
    pub const fn guest(&self) -> &BoundedPageSetCheckpointComparison {
        &self.guest
    }

    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.guest.page_exact(address)
    }

    #[must_use]
    pub fn vcpu_exact(&self) -> bool {
        self.guest.vcpu().is_exact_match()
    }

    #[must_use]
    pub const fn device_exact(&self) -> bool {
        self.device
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.guest.is_exact_match() && self.device
    }
}
