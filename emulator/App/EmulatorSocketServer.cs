using System.Net.Sockets;
using Kb.HostProtocol;

namespace KbEmulator;

/// <summary>
/// Accepts Raw HID packets from <c>kbnotify</c> (or anything else) on a Unix
/// domain socket, so the OS side can be tested against the emulator.
/// </summary>
public sealed class EmulatorSocketServer : IDisposable
{
    private readonly string _path;
    private readonly Action<byte[]> _onPacket;
    private readonly CancellationTokenSource _stop = new();
    private Socket? _listener;

    public EmulatorSocketServer(Action<byte[]> onPacket, string? path = null)
    {
        _onPacket = onPacket;
        _path = path ?? EmulatorLink.DefaultSocketPath;
    }

    public string Path => _path;

    /// <summary>Start listening. Returns false when another emulator already owns the socket.</summary>
    public bool Start()
    {
        if (File.Exists(_path))
        {
            if (IsInUse()) return false;
            File.Delete(_path); // stale socket from a crashed run
        }
        _listener = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified);
        _listener.Bind(new UnixDomainSocketEndPoint(_path));
        _listener.Listen();
        _ = AcceptLoop(_listener, _stop.Token);
        return true;
    }

    private bool IsInUse()
    {
        using var probe = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified);
        try
        {
            probe.Connect(new UnixDomainSocketEndPoint(_path));
            return true;
        }
        catch (SocketException)
        {
            return false;
        }
    }

    private async Task AcceptLoop(Socket listener, CancellationToken stop)
    {
        while (!stop.IsCancellationRequested)
        {
            Socket client;
            try
            {
                client = await listener.AcceptAsync(stop).ConfigureAwait(false);
            }
            catch (Exception e) when (stop.IsCancellationRequested && e is SocketException or OperationCanceledException or ObjectDisposedException)
            {
                return;
            }
            _ = ReadPackets(client, stop);
        }
    }

    private async Task ReadPackets(Socket client, CancellationToken stop)
    {
        using (client)
        {
            var packet = new byte[NotificationPackets.ReportSize];
            while (!stop.IsCancellationRequested)
            {
                var filled = 0;
                while (filled < packet.Length)
                {
                    int n;
                    try
                    {
                        n = await client.ReceiveAsync(packet.AsMemory(filled), SocketFlags.None, stop).ConfigureAwait(false);
                    }
                    catch (Exception)
                    {
                        return;
                    }
                    if (n == 0) return; // client closed
                    filled += n;
                }
                _onPacket(packet.ToArray());
            }
        }
    }

    public void Dispose()
    {
        _stop.Cancel();
        _listener?.Dispose();
        if (_listener != null && File.Exists(_path)) File.Delete(_path);
        _stop.Dispose();
    }
}
