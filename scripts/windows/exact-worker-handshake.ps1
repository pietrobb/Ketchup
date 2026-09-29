function Assert-ExactWorkerHandshake {
    <# Sends the typed Hello frame (u32 LE length + CBOR) and checks the Hello reply. #>
    param([Parameter(Mandatory)] [string] $WorkerPath, [Parameter(Mandatory)] [string] $Context)

    $start = [System.Diagnostics.ProcessStartInfo]::new($WorkerPath)
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $process = [System.Diagnostics.Process]::Start($start)
    $hello = [byte[]](0x65) + [System.Text.Encoding]::ASCII.GetBytes("Hello")
    $frame = [System.BitConverter]::GetBytes([uint32]$hello.Length) + $hello
    $process.StandardInput.BaseStream.Write($frame, 0, $frame.Length)
    $process.StandardInput.Close()
    $buffer = [System.IO.MemoryStream]::new()
    $process.StandardOutput.BaseStream.CopyTo($buffer)
    $process.WaitForExit()
    $reply = $buffer.ToArray()
    # {"Hello": {"protocol": <64-character text>}}
    $prefix = [byte[]](0xa1, 0x65) + [System.Text.Encoding]::ASCII.GetBytes("Hello") +
        [byte[]](0xa1, 0x68) + [System.Text.Encoding]::ASCII.GetBytes("protocol") + [byte[]](0x78, 0x40)
    $valid = $process.ExitCode -eq 0 -and $reply.Length -eq 4 + $prefix.Length + 64 -and
        [System.BitConverter]::ToUInt32($reply, 0) -eq $reply.Length - 4
    for ($index = 0; $valid -and $index -lt $prefix.Length; $index++) {
        $valid = $reply[4 + $index] -eq $prefix[$index]
    }
    if (-not $valid) {
        throw "$Context failed its exact worker Hello handshake."
    }
}
