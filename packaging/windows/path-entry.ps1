# Adds or removes one directory in the machine-wide PATH.
# Usage: powershell -File path-entry.ps1 add|remove <dir>
# The Path value is read without expanding %VAR% references and written back as REG_EXPAND_SZ,
# so entries such as %SystemRoot%\system32 survive and there is no length limit.
# Dot-sourcing this file only defines the pure functions below (used by path-entry.tests.ps1).
param(
    [string]$Action = '',
    [string]$Dir = ''
)

function ConvertTo-PathKey([string]$Segment) {
    $s = $Segment.Trim()
    $s = $s.Trim('"').Trim()
    $s = $s.TrimEnd('\')
    return $s.ToLowerInvariant()
}

function Add-PathEntry([string]$Path, [string]$Dir) {
    $key = ConvertTo-PathKey $Dir
    foreach ($seg in $Path.Split(';')) {
        if ($seg -ne '' -and (ConvertTo-PathKey $seg) -eq $key) {
            return [string]$Path
        }
    }
    $base = $Path.TrimEnd(';')
    if ($base -eq '') {
        return [string]$Dir
    }
    return [string]($base + ';' + $Dir)
}

function Remove-PathEntry([string]$Path, [string]$Dir) {
    $key = ConvertTo-PathKey $Dir
    $kept = New-Object System.Collections.Generic.List[string]
    $removed = $false
    foreach ($seg in $Path.Split(';')) {
        if ($seg -ne '' -and (ConvertTo-PathKey $seg) -eq $key) {
            $removed = $true
        } else {
            $kept.Add($seg)
        }
    }
    if (-not $removed) {
        return [string]$Path
    }
    return [string]($kept -join ';')
}

function Update-MachinePath([string]$Action, [string]$Dir) {
    $sub = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
    $key = $base.OpenSubKey($sub, $true)
    try {
        $old = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if ($Action -eq 'add') { $new = Add-PathEntry $old $Dir } else { $new = Remove-PathEntry $old $Dir }
        if ($new -ne $old) {
            $key.SetValue('Path', $new, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        }
    } finally {
        $key.Close()
        $base.Close()
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        if (($Action -ne 'add' -and $Action -ne 'remove') -or $Dir -eq '') {
            throw 'usage: path-entry.ps1 add|remove <dir>'
        }
        Update-MachinePath $Action $Dir
    } catch {
        [Console]::Error.WriteLine($_.Exception.Message)
        exit 1
    }
}
