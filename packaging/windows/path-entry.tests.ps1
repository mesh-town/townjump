# TAP tests for the pure functions of path-entry.ps1.
# Run: pwsh -NoProfile -NonInteractive -File packaging/windows/path-entry.tests.ps1
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'path-entry.ps1')

$script:n = 0
$script:failed = 0
function Check([string]$Name, [string]$Got, [string]$Want) {
    $script:n++
    if ($Got -ceq $Want) {
        Write-Output "ok $($script:n) - $Name"
    } else {
        $script:failed++
        Write-Output "not ok $($script:n) - $Name"
        Write-Output "# got:  [$Got]"
        Write-Output "# want: [$Want]"
    }
}

$d = 'C:\Program Files\Townjump'

Check 'add: empty path' (Add-PathEntry '' $d) $d
Check 'add: appends after existing' (Add-PathEntry 'A;B' $d) "A;B;$d"
Check 'add: trailing semicolon, no double' (Add-PathEntry 'A;' $d) "A;$d"
Check 'add: present with other case' (Add-PathEntry 'A;c:\program files\TOWNJUMP;B' $d) 'A;c:\program files\TOWNJUMP;B'
Check 'add: present with trailing backslash' (Add-PathEntry "A;$d\" $d) "A;$d\"
Check 'add: present in quotes' (Add-PathEntry "A;`"$d`"" $d) "A;`"$d`""
Check 'add: present with outer spaces' (Add-PathEntry "A; $d ;B" $d) "A; $d ;B"
Check 'add: %VAR% entry stays unexpanded' (Add-PathEntry '%SystemRoot%\system32;A' $d) "%SystemRoot%\system32;A;$d"
Check 'add: common prefix is not a match' (Add-PathEntry "A;${d}2" $d) "A;${d}2;$d"

$long = ((1..500) | ForEach-Object { "C:\dir$_" }) -join ';'
$long = $long + ';' + ('x' * 100)
$longOut = Add-PathEntry $long $d
Check 'add: path over 5000 chars is not truncated' ("$($longOut.Length)|$($longOut.StartsWith($long))|$($longOut.EndsWith($d))") "$($long.Length + 1 + $d.Length)|True|True"
Check 'add: long path length precondition' ([string]($long.Length -gt 1024)) 'True'

Check 'remove: no entry, unchanged' (Remove-PathEntry 'A;B' $d) 'A;B'
Check 'remove: empty path' (Remove-PathEntry '' $d) ''
Check 'remove: middle entry' (Remove-PathEntry "A;$d;B" $d) 'A;B'
Check 'remove: all entries, any spelling' (Remove-PathEntry "$d;A;$d\" $d) 'A'
Check 'remove: quoted and cased entry' (Remove-PathEntry "A;`"C:\PROGRAM FILES\townjump`";B" $d) 'A;B'
Check 'remove: common prefix is kept' (Remove-PathEntry "A;${d}2;B" $d) "A;${d}2;B"
Check 'remove: %VAR% entries kept verbatim' (Remove-PathEntry "%SystemRoot%\system32;$d;%ProgramFiles%\x" $d) '%SystemRoot%\system32;%ProgramFiles%\x'
Check 'remove: only entry leaves empty' (Remove-PathEntry $d $d) ''

$self = (Get-Process -Id $PID).Path
$null = & $self -NoProfile -NonInteractive -File (Join-Path $PSScriptRoot 'path-entry.ps1') 'bogus' 'x' 2>$null
Check 'direct run: bad action exits 1' ([string]$LASTEXITCODE) '1'
$null = & $self -NoProfile -NonInteractive -File (Join-Path $PSScriptRoot 'path-entry.ps1') 'add' 2>$null
Check 'direct run: missing dir exits 1' ([string]$LASTEXITCODE) '1'

Write-Output "1..$($script:n)"
if ($script:failed -gt 0) { exit 1 }
exit 0
