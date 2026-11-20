//! The radio the TOWER gateway is written against (T24): the north bridge's firmware gives it
//! over `fiber-subghz` and the [`packet`](crate::packet) codec, the gateway's tests over a radio
//! made up in software. What goes through it is a TOWER packet's payload: the network frame.

/// What came with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Received {
    /// The frame's length in the caller's buffer.
    pub len: usize,
    /// The signal strength at its sync word, in dBm. The STM32WL gives nothing like the SPIRIT1's
    /// link quality numbers, so this is all there is (`docs/firmware/radio.md`).
    pub rssi: i16,
}

/// One radio, on one channel at a time.
///
/// The futures are not `Send`: the north bridge's firmware runs on one executor, on one core.
#[allow(async_fn_in_trait)]
pub trait Radio {
    type Error;

    /// The channel both sending and receiving use: 0 to 2 in EU 868 (868.1, 868.3, 868.5 MHz).
    fn set_channel(&mut self, channel: u8) -> Result<(), Self::Error>;

    /// Sends `frame` as one packet, and returns once it is on air.
    async fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error>;

    /// Waits for the next packet whose CRC is right, and puts its frame into `buf`; `None` when
    /// no packet began before `until_ms` (the gateway's [`Clock`](crate::gateway::Clock) time).
    /// A packet that began before it is received to its end: a deadline that cut one off would
    /// lose it, and a node's tries come 200 ms or more apart.
    async fn receive(&mut self, buf: &mut [u8], until_ms: u64) -> Result<Option<Received>, Self::Error>;
}
