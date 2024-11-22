use embedded_graphics_core::pixelcolor::{Rgb565, Rgb666};
use embedded_hal::{digital::OutputPin, spi::SpiDevice};

use super::{CommandInterface, PixelInterface};

/// Spi interface error
#[derive(Clone, Copy, Debug)]
pub enum SpiError<SPI, DC> {
    /// SPI bus error
    Spi(SPI),
    /// Data/command pin error
    Dc(DC),
}

/// Spi interface
pub struct SpiInterface<'a, SPI, DC> {
    spi: BufferedSpiAdapter<'a, SPI>,
    dc: DC,
}

impl<'a, SPI: SpiDevice, DC: OutputPin> SpiInterface<'a, SPI, DC> {
    /// Create new interface
    pub fn new(spi: SPI, dc: DC, buffer: &'a mut [u8]) -> Self {
        let spi = BufferedSpiAdapter::new(spi, buffer);
        Self { spi, dc }
    }
}

impl<SPI: SpiDevice, DC: OutputPin> CommandInterface for SpiInterface<'_, SPI, DC> {
    type Error = SpiError<SPI::Error, DC::Error>;

    fn send_command(&mut self, command: u8, args: &[u8]) -> Result<(), Self::Error> {
        self.flush()?;
        self.dc.set_low().map_err(SpiError::Dc)?;
        self.spi.push_bytes(&[command]).map_err(SpiError::Spi)?;
        self.spi.flush().map_err(SpiError::Spi)?;
        self.dc.set_high().map_err(SpiError::Dc)?;
        self.spi.push_bytes(args).map_err(SpiError::Spi)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.spi.flush().map_err(SpiError::Spi)
    }
}

fn rgb565_to_bytes(pixel: Rgb565) -> [u8; 2] {
    embedded_graphics_core::pixelcolor::raw::ToBytes::to_be_bytes(pixel)
}
fn rgb666_to_bytes(pixel: Rgb666) -> [u8; 3] {
    embedded_graphics_core::pixelcolor::raw::ToBytes::to_be_bytes(pixel).map(|x| x << 2)
}

impl<SPI: SpiDevice, DC: OutputPin> PixelInterface<Rgb565> for SpiInterface<'_, SPI, DC> {
    fn send_repeated_pixel(&mut self, pixel: Rgb565, count: u32) -> Result<(), Self::Error> {
        self.spi
            .push_bytes_repeated(&rgb565_to_bytes(pixel), count)
            .map_err(SpiError::Spi)
    }

    fn send_pixels(&mut self, pixels: impl IntoIterator<Item = Rgb565>) -> Result<(), Self::Error> {
        self.spi
            .push_array_iter(pixels.into_iter().map(rgb565_to_bytes))
            .map_err(SpiError::Spi)
    }
}

impl<SPI: SpiDevice, DC: OutputPin> PixelInterface<Rgb666> for SpiInterface<'_, SPI, DC> {
    fn send_repeated_pixel(&mut self, pixel: Rgb666, count: u32) -> Result<(), Self::Error> {
        self.spi
            .push_bytes_repeated(&rgb666_to_bytes(pixel), count)
            .map_err(SpiError::Spi)
    }

    fn send_pixels(&mut self, pixels: impl IntoIterator<Item = Rgb666>) -> Result<(), Self::Error> {
        self.spi
            .push_array_iter(pixels.into_iter().map(rgb666_to_bytes))
            .map_err(SpiError::Spi)
    }
}

struct BufferedSpiAdapter<'a, SPI> {
    spi: SPI,
    buffer: &'a mut [u8],
    index: usize,
}

impl<'a, SPI: SpiDevice> BufferedSpiAdapter<'a, SPI> {
    fn new(spi: SPI, buffer: &'a mut [u8]) -> Self {
        Self {
            spi,
            buffer,
            index: 0,
        }
    }

    fn fill_buffer(&mut self, filler: impl FnOnce(&mut [u8]) -> usize) -> Result<(), SPI::Error> {
        if self.index == self.buffer.len() {
            self.flush()?;
        }
        let buffer = &mut self.buffer[self.index..];
        self.index += filler(buffer);
        assert!(self.index <= self.buffer.len());
        Ok(())
    }

    fn flush(&mut self) -> Result<(), SPI::Error> {
        let index = core::mem::replace(&mut self.index, 0);
        if index != 0 {
            self.spi.write(&self.buffer[0..index])?;
        }
        Ok(())
    }

    fn push_bytes_repeated(&mut self, bytes: &[u8], count: u32) -> Result<(), SPI::Error> {
        {
            let this = &mut *self;
            let mut count = count;
            if let Ok(count_bytes) = usize::try_from(count * bytes.len() as u32) {
                if count_bytes <= this.buffer.len() - this.index {
                    // There is enough remaining space in the buffer for all the new data
                    for i in 0..count as usize {
                        this.buffer[(this.index + (i * bytes.len()))..][..bytes.len()]
                            .copy_from_slice(bytes);
                    }
                    this.index += count_bytes;
                    return Ok(());
                }
            }

            this.flush()?;

            // let buffer_len = self.buffer.len() / bytes.len();

            let fill_count = core::cmp::min(count as usize, this.buffer.len() / bytes.len());

            for i in 0..fill_count {
                this.buffer[i * bytes.len()..][..bytes.len()].copy_from_slice(bytes);
            }

            while count >= fill_count as u32 {
                this.index = fill_count * bytes.len();
                this.flush()?;

                count -= fill_count as u32;
            }

            this.index = count as usize * bytes.len();

            Ok(())
        }
    }

    fn push_bytes(&mut self, mut bytes: &[u8]) -> Result<(), SPI::Error> {
        while !bytes.is_empty() {
            self.fill_buffer(|buffer| {
                let len = core::cmp::min(buffer.len(), bytes.len());
                let (to_send, remainder) = bytes.split_at(len);
                bytes = remainder;
                buffer[0..len].copy_from_slice(to_send);
                len
            })?;
        }
        Ok(())
    }

    fn push_array_iter<const N: usize>(
        &mut self,
        arrays: impl IntoIterator<Item = [u8; N]>,
    ) -> Result<(), SPI::Error> {
        let mut arrays = arrays.into_iter();

        let mut done = false;
        while !done {
            self.fill_buffer(|buffer| {
                let mut i = 0;
                // TODO: make sure buffer will hold at least one chunk
                for chunk in buffer.chunks_exact_mut(N) {
                    if let Some(array) = arrays.next() {
                        chunk.copy_from_slice(&array);
                        i += N;
                    } else {
                        done = true;
                        break;
                    };
                }
                i
            })?;
        }
        Ok(())
    }
}
