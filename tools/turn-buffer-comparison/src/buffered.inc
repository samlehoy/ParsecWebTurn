// Isolated candidate using the BUD policy, with the unavoidable Conn output copy.
struct BufferedReader {
    inner: ReadHalf<Stream>,
    bytes: Box<[u8]>,
    start: usize,
    end: usize,
}
impl BufferedReader {
    fn new(inner: ReadHalf<Stream>) -> Self {
        Self { inner, bytes: vec![0; 64 * 1024].into_boxed_slice(), start: 0, end: 0 }
    }
    async fn receive(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.end - self.start >= 4 {
                let (size, pad) = frame_shape(&self.bytes[self.start..])?;
                if size > destination.len() || size + pad > self.bytes.len() { return Err(invalid()); }
                if self.end - self.start >= size + pad {
                    let frame = &self.bytes[self.start..self.start + size];
                    if frame[0] >> 6 == 0 && frame[4..8] != [0x21,0x12,0xa4,0x42] { return Err(invalid()); }
                    destination[..size].copy_from_slice(frame);
                    self.start += size + pad;
                    return Ok(size);
                }
            }
            if self.end == self.bytes.len() || self.start == self.end {
                self.bytes.copy_within(self.start..self.end, 0);
                self.end -= self.start;
                self.start = 0;
            }
            let count = self.inner.read(&mut self.bytes[self.end..]).await?;
            if count == 0 { return Err(io::ErrorKind::UnexpectedEof.into()); }
            self.end += count;
        }
    }
}

struct BufferedWriter { inner: WriteHalf<Stream>, staging: Box<[u8]> }
impl BufferedWriter {
    fn new(inner: WriteHalf<Stream>) -> Self {
        Self { inner, staging: vec![0;32*1024].into_boxed_slice() }
    }
}
impl AsyncWrite for BufferedWriter {
    fn poll_write(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, bytes: &[u8]) -> std::task::Poll<io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx,bytes)
    }
    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl TurnStream {
    pub async fn send_ready(&self, messages: &[Vec<u8>]) -> util::Result<()> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() { return Err(io::ErrorKind::NotConnected.into()); }
        tokio::select! {
            biased;
            _ = closed.changed() => Err(io::ErrorKind::NotConnected.into()),
            result = async {
                let mut writer = self.writer.lock().await;
                let BufferedWriter { inner, staging } = &mut *writer;
                let mut used = 0;
                for message in messages {
                    let (size, pad) = frame_shape(message)?;
                    if message.len() != size || size + pad > staging.len() { return Err(invalid()); }
                    if staging.len() - used < size + pad {
                        inner.write_all(&staging[..used]).await?;
                        inner.flush().await?;
                        used = 0;
                    }
                    staging[used..used+size].copy_from_slice(message);
                    staging[used+size..used+size+pad].fill(0);
                    used += size + pad;
                }
                if used != 0 {
                    inner.write_all(&staging[..used]).await?;
                    inner.flush().await?;
                }
                Ok::<_,io::Error>(())
            } => {
                if result.is_err() { self.closed.send_replace(true); }
                result.map_err(Into::into)
            }
        }
    }
}
