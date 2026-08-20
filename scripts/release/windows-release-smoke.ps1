<#
.SYNOPSIS
Runs an isolated Windows startup and clean-shutdown smoke test for a freshly built executable.

.DESCRIPTION
Starts the unpacked application executable without installing an NSIS bundle. The test creates a
uniquely named, marker-gated runtime root and supplies VHM_RELEASE_SMOKE_ROOT so Tauri's database
and thumbnail cache are redirected away from the signed-in user's real Known Folder locations.
WEBVIEW2_USER_DATA_FOLDER is also set below that root so WebView2 is isolated before any Tauri setup
callback runs; the application independently applies the same path to its explicit window builder.
TEMP and TMP live in a separate marker-gated sibling. Windows Known Folder APIs do not provide a
safe isolation boundary through APPDATA, LOCALAPPDATA, USERPROFILE, HOME, HOMEDRIVE, or HOMEPATH,
so those variables are inherited unchanged. The explicit Rust smoke root remains authoritative for
the database, thumbnail cache, and WebView2 data directory.

The executable is created suspended, assigned to a Windows Job configured with KILL_ON_JOB_CLOSE,
and only then resumed. An explicit inherited-handle list redirects child stdout and stderr into the
marker-gated environment root; failures include a bounded tail of both streams. The test fails
closed unless it can prove Job containment, path containment, database schema v18 (including empty
trash-snapshot and delete-intent journals), an empty fresh custom-tag catalog, absence of scans, single-instance
handoff to the original visible window, UI process survival, graceful exit code 0, an empty Job
after shutdown, unchanged real application data/cache trees, and safe cleanup.

Pass only a freshly built executable. This script deliberately does not install or execute an NSIS
installer and does not infer freshness from an old target/release directory.

.PARAMETER ExecutablePath
Path to the fresh unpacked Windows application executable.

.PARAMETER ResourceDirectory
Path to the unpacked application resource directory.

.PARAMETER FfmpegMode
Unavailable (default) points VHM_FFMPEG_PATH at a guaranteed-missing isolated path so startup
cannot spawn FFmpeg. ResourceOverride points it at ResourceDirectory\bin\ffmpeg.exe.

.PARAMETER IsolationParent
Existing directory under which the uniquely named marker-gated smoke root is created.

.PARAMETER SecondInstanceTimeoutSeconds
Maximum time allowed for the second launch to hand off to the original process and exit.

.PARAMETER ExpectedExecutableSha256
Optional expected SHA-256 for the exact executable being launched.

.EXAMPLE
pwsh ./scripts/release/windows-release-smoke.ps1 `
  -ExecutablePath ./artifacts/app/valorant-highlight-manager.exe `
  -ResourceDirectory ./artifacts/app `
  -ExpectedExecutableSha256 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef

.EXAMPLE
pwsh ./scripts/release/windows-release-smoke.ps1 `
  -ExecutablePath ./artifacts/app/valorant-highlight-manager.exe `
  -ResourceDirectory ./artifacts/app `
  -FfmpegMode ResourceOverride
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExecutablePath,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ResourceDirectory,

    [ValidateSet('Unavailable', 'ResourceOverride')]
    [string] $FfmpegMode = 'Unavailable',

    [ValidateNotNullOrEmpty()]
    [string] $IsolationParent = $(
        if (-not [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
            $env:RUNNER_TEMP
        }
        else {
            [System.IO.Path]::GetTempPath()
        }
    ),

    [ValidateRange(5, 180)]
    [int] $StartupTimeoutSeconds = 45,

    [ValidateRange(2, 30)]
    [int] $SecondInstanceTimeoutSeconds = 5,

    [ValidateRange(2, 60)]
    [int] $ShutdownTimeoutSeconds = 15,

    [ValidateRange(1, 30)]
    [int] $DatabaseInspectionTimeoutSeconds = 10,

    [ValidateNotNullOrEmpty()]
    [string] $AppIdentifier = 'com.valorant.highlight.manager',

    [string] $PythonExecutablePath,

    [string] $ExpectedExecutableSha256
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$SmokeRootPrefix = 'vhm-release-smoke-'
$SmokeMarkerName = '.vhm-release-smoke-root'
$SmokeMarkerContent = 'vhm-release-smoke-root-v1'
$EnvironmentRootPrefix = 'vhm-release-smoke-env-'
$EnvironmentMarkerName = '.vhm-release-smoke-env-root'
$EnvironmentMarkerContent = 'vhm-release-smoke-env-root-v1'
$ExpectedSchemaVersion = 18
$RequiredTables = @(
    'source_dirs',
    'clip_groups',
    'clips',
    'clip_thumbnails',
    'clip_metadata',
    'matches',
    'match_stats',
    'match_snapshots',
    'match_events',
    'clip_segments',
    'clip_events',
    'tags',
    'clip_tags',
    'scan_runs',
    'clip_trash_snapshots',
    'clip_delete_intents'
)
$RequiredColumns = [ordered]@{
    clips = @(
        'file_volume_serial',
        'file_index_high',
        'file_index_low'
    )
    clip_events = @('killed_is_me')
    scan_runs = @('summary_available')
}
function Test-IsWindows {
    return [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [System.Runtime.InteropServices.OSPlatform]::Windows
    )
}

function Get-CanonicalExistingPath {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [bool] $RequireDirectory,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    $resolvedPaths = @(Resolve-Path -LiteralPath $LiteralPath -ErrorAction Stop)
    if ($resolvedPaths.Count -ne 1) {
        throw "$Description must resolve to exactly one path: '$LiteralPath'."
    }
    $resolved = $resolvedPaths[0]
    $path = [System.IO.Path]::GetFullPath($resolved.ProviderPath)
    $item = Get-Item -LiteralPath $path -Force -ErrorAction Stop
    if ($RequireDirectory -and -not $item.PSIsContainer) {
        throw "$Description must be a directory: '$path'."
    }
    if (-not $RequireDirectory -and $item.PSIsContainer) {
        throw "$Description must be a regular file: '$path'."
    }
    if (-not $RequireDirectory -and $item.Length -le 0) {
        throw "$Description must not be empty: '$path'."
    }
    return $path
}

function Test-PathWithinRoot {
    param(
        [Parameter(Mandatory = $true)] [string] $Root,
        [Parameter(Mandatory = $true)] [string] $Candidate,
        [switch] $AllowRoot
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $candidateFull = [System.IO.Path]::GetFullPath($Candidate).TrimEnd('\', '/')
    if ($AllowRoot -and [string]::Equals(
            $rootFull,
            $candidateFull,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        return $true
    }
    return $candidateFull.StartsWith(
        $rootFull + [System.IO.Path]::DirectorySeparatorChar,
        [System.StringComparison]::OrdinalIgnoreCase
    )
}

function Test-PathsOverlap {
    param(
        [Parameter(Mandatory = $true)] [string] $First,
        [Parameter(Mandatory = $true)] [string] $Second
    )

    return (Test-PathWithinRoot -Root $First -Candidate $Second -AllowRoot) -or
        (Test-PathWithinRoot -Root $Second -Candidate $First -AllowRoot)
}

function Assert-Sha256Text {
    param(
        [Parameter(Mandatory = $true)] [string] $Value,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    if ($Value -notmatch '^[0-9a-fA-F]{64}$') {
        throw "$Description must be a 64-character hexadecimal SHA-256 value."
    }
}

function Get-Sha256 {
    param([Parameter(Mandatory = $true)] [string] $LiteralPath)
    return (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
}

function Assert-PeFile {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    $stream = [System.IO.File]::Open(
        $LiteralPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    try {
        if ($stream.Length -lt 64) {
            throw "$Description is too small to be a Windows PE file: '$LiteralPath'."
        }
        if ($stream.ReadByte() -ne 0x4d -or $stream.ReadByte() -ne 0x5a) {
            throw "$Description does not have an MZ header: '$LiteralPath'."
        }

        $stream.Position = 0x3c
        $offsetBytes = [byte[]]::new(4)
        if ($stream.Read($offsetBytes, 0, 4) -ne 4) {
            throw "$Description has a truncated PE header offset."
        }
        $peOffset = [System.BitConverter]::ToUInt32($offsetBytes, 0)
        if ($peOffset -gt ($stream.Length - 4)) {
            throw "$Description has an invalid PE header offset."
        }
        $stream.Position = $peOffset
        $signature = [byte[]]::new(4)
        if ($stream.Read($signature, 0, 4) -ne 4 -or
            $signature[0] -ne 0x50 -or $signature[1] -ne 0x45 -or
            $signature[2] -ne 0 -or $signature[3] -ne 0) {
            throw "$Description does not have a valid PE signature."
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Add-KnownFolderInterop {
    if ('VhmReleaseSmoke.KnownFolders' -as [type]) {
        return
    }

    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace VhmReleaseSmoke {
    public static class KnownFolders {
        [DllImport("shell32.dll")]
        private static extern int SHGetKnownFolderPath(
            [MarshalAs(UnmanagedType.LPStruct)] Guid rfid,
            uint dwFlags,
            IntPtr hToken,
            out IntPtr ppszPath);

        public static string GetPath(string folderId) {
            IntPtr rawPath;
            int result = SHGetKnownFolderPath(new Guid(folderId), 0, IntPtr.Zero, out rawPath);
            if (result != 0) {
                Marshal.ThrowExceptionForHR(result);
            }
            try {
                return Marshal.PtrToStringUni(rawPath);
            }
            finally {
                Marshal.FreeCoTaskMem(rawPath);
            }
        }
    }
}
'@
}

function Add-WindowInterop {
    if ('VhmReleaseSmoke.NativeWindows' -as [type]) {
        return
    }

    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace VhmReleaseSmoke {
    public sealed class WindowInfo {
        public IntPtr Handle { get; private set; }
        public uint ProcessId { get; private set; }
        public uint ThreadId { get; private set; }
        public bool Visible { get; private set; }
        public bool Minimized { get; private set; }
        public IntPtr OwnerHandle { get; private set; }
        public string Title { get; private set; }
        public string ClassName { get; private set; }

        internal WindowInfo(
            IntPtr handle,
            uint processId,
            uint threadId,
            bool visible,
            bool minimized,
            IntPtr ownerHandle,
            string title,
            string className) {
            Handle = handle;
            ProcessId = processId;
            ThreadId = threadId;
            Visible = visible;
            Minimized = minimized;
            OwnerHandle = ownerHandle;
            Title = title;
            ClassName = className;
        }
    }

    public static class NativeWindows {
        private delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
        private const uint WM_CLOSE = 0x0010;
        private const uint GW_OWNER = 4;
        private const int SW_MINIMIZE = 6;

        [DllImport("user32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);

        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern int GetWindowTextW(IntPtr window, StringBuilder text, int maximumCount);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern int GetWindowTextLengthW(IntPtr window);

        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern int GetClassNameW(IntPtr window, StringBuilder className, int maximumCount);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool IsWindowVisible(IntPtr window);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool IsIconic(IntPtr window);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool IsWindow(IntPtr window);

        [DllImport("user32.dll")]
        private static extern IntPtr GetWindow(IntPtr window, uint command);

        [DllImport("user32.dll", EntryPoint = "PostMessageW", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool ShowWindow(IntPtr window, int command);

        [DllImport("user32.dll")]
        private static extern IntPtr GetForegroundWindow();

        public static WindowInfo[] ForProcess(int expectedProcessId) {
            var windows = new List<WindowInfo>();
            bool enumerated = EnumWindows((window, parameter) => {
                uint processId;
                GetWindowThreadProcessId(window, out processId);
                if (processId == (uint) expectedProcessId) {
                    WindowInfo information = Describe(window);
                    if (information != null) {
                        windows.Add(information);
                    }
                }
                return true;
            }, IntPtr.Zero);
            if (!enumerated) {
                int error = Marshal.GetLastWin32Error();
                // A non-interactive window station can report FALSE with ERROR_SUCCESS when it
                // simply exposes no top-level windows. Treat the empty inventory as evidence;
                // real Win32 errors still fail closed.
                if (error != 0) {
                    throw new Win32Exception(error, "EnumWindows failed");
                }
            }
            return windows.ToArray();
        }

        public static WindowInfo Describe(IntPtr window) {
            if (window == IntPtr.Zero || !IsWindow(window)) {
                return null;
            }
            uint processId;
            uint threadId = GetWindowThreadProcessId(window, out processId);
            int titleLength = GetWindowTextLengthW(window);
            StringBuilder title = new StringBuilder(Math.Max(1, titleLength + 1));
            GetWindowTextW(window, title, title.Capacity);
            StringBuilder className = new StringBuilder(512);
            GetClassNameW(window, className, className.Capacity);
            return new WindowInfo(
                window,
                processId,
                threadId,
                IsWindowVisible(window),
                IsIconic(window),
                GetWindow(window, GW_OWNER),
                title.ToString(),
                className.ToString());
        }

        public static bool ExistsForProcess(IntPtr window, int expectedProcessId) {
            if (window == IntPtr.Zero || !IsWindow(window)) {
                return false;
            }
            uint processId;
            GetWindowThreadProcessId(window, out processId);
            return processId == (uint) expectedProcessId;
        }

        public static bool RequestClose(IntPtr window) {
            return window != IntPtr.Zero && PostMessage(window, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
        }

        public static void Minimize(IntPtr window) {
            if (window != IntPtr.Zero) {
                ShowWindow(window, SW_MINIMIZE);
            }
        }

        public static IntPtr ForegroundWindow() {
            return GetForegroundWindow();
        }
    }
}
'@
}

function Add-JobProcessInterop {
    if ('VhmReleaseSmoke.JobProcess' -as [type]) {
        return
    }

    Add-Type -TypeDefinition @'
using System;
using System.Collections;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

namespace VhmReleaseSmoke {
    public sealed class JobAccounting {
        public uint TotalProcesses { get; private set; }
        public uint ActiveProcesses { get; private set; }
        public uint TotalTerminatedProcesses { get; private set; }

        internal JobAccounting(uint total, uint active, uint terminated) {
            TotalProcesses = total;
            ActiveProcesses = active;
            TotalTerminatedProcesses = terminated;
        }
    }

    public sealed class JobProcess : IDisposable {
        private const uint CREATE_SUSPENDED = 0x00000004;
        private const uint CREATE_UNICODE_ENVIRONMENT = 0x00000400;
        private const uint EXTENDED_STARTUPINFO_PRESENT = 0x00080000;
        private const uint STARTF_USESHOWWINDOW = 0x00000001;
        private const uint STARTF_USESTDHANDLES = 0x00000100;
        private const short SW_HIDE = 0;
        private const uint GENERIC_READ = 0x80000000;
        private const uint GENERIC_WRITE = 0x40000000;
        private const uint FILE_SHARE_READ = 0x00000001;
        private const uint FILE_SHARE_WRITE = 0x00000002;
        private const uint FILE_SHARE_DELETE = 0x00000004;
        private const uint CREATE_ALWAYS = 2;
        private const uint OPEN_EXISTING = 3;
        private const uint FILE_ATTRIBUTE_NORMAL = 0x00000080;
        private const int ERROR_INSUFFICIENT_BUFFER = 122;
        private const int ERROR_MORE_DATA = 234;
        private static readonly IntPtr PROC_THREAD_ATTRIBUTE_HANDLE_LIST = new IntPtr(0x00020002);
        private static readonly IntPtr INVALID_HANDLE_VALUE = new IntPtr(-1);
        private const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
        private const int JobObjectBasicAccountingInformation = 1;
        private const int JobObjectBasicProcessIdList = 3;
        private const int JobObjectExtendedLimitInformation = 9;
        private const uint WAIT_OBJECT_0 = 0;
        private const uint WAIT_TIMEOUT = 258;
        private const uint WAIT_FAILED = 0xffffffff;
        private const uint STILL_ACTIVE = 259;

        [StructLayout(LayoutKind.Sequential)]
        private struct SECURITY_ATTRIBUTES {
            public uint nLength;
            public IntPtr lpSecurityDescriptor;
            [MarshalAs(UnmanagedType.Bool)]
            public bool bInheritHandle;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct STARTUPINFO {
            public uint cb;
            public IntPtr lpReserved;
            public IntPtr lpDesktop;
            public IntPtr lpTitle;
            public uint dwX;
            public uint dwY;
            public uint dwXSize;
            public uint dwYSize;
            public uint dwXCountChars;
            public uint dwYCountChars;
            public uint dwFillAttribute;
            public uint dwFlags;
            public short wShowWindow;
            public short cbReserved2;
            public IntPtr lpReserved2;
            public IntPtr hStdInput;
            public IntPtr hStdOutput;
            public IntPtr hStdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct STARTUPINFOEX {
            public STARTUPINFO StartupInfo;
            public IntPtr lpAttributeList;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct PROCESS_INFORMATION {
            public IntPtr hProcess;
            public IntPtr hThread;
            public uint dwProcessId;
            public uint dwThreadId;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
            public long PerProcessUserTimeLimit;
            public long PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct IO_COUNTERS {
            public ulong ReadOperationCount;
            public ulong WriteOperationCount;
            public ulong OtherOperationCount;
            public ulong ReadTransferCount;
            public ulong WriteTransferCount;
            public ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
            public IO_COUNTERS IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION {
            public long TotalUserTime;
            public long TotalKernelTime;
            public long ThisPeriodTotalUserTime;
            public long ThisPeriodTotalKernelTime;
            public uint TotalPageFaultCount;
            public uint TotalProcesses;
            public uint ActiveProcesses;
            public uint TotalTerminatedProcesses;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateJobObjectW(IntPtr jobAttributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SetInformationJobObject(
            IntPtr job,
            int informationClass,
            IntPtr information,
            uint informationLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool QueryInformationJobObject(
            IntPtr job,
            int informationClass,
            IntPtr information,
            uint informationLength,
            out uint returnLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool TerminateJobObject(IntPtr job, uint exitCode);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CreateProcessW(
            string applicationName,
            StringBuilder commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            [MarshalAs(UnmanagedType.Bool)] bool inheritHandles,
            uint creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref STARTUPINFOEX startupInfo,
            out PROCESS_INFORMATION processInformation);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateFileW(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            ref SECURITY_ATTRIBUTES securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool InitializeProcThreadAttributeList(
            IntPtr attributeList,
            int attributeCount,
            uint flags,
            ref UIntPtr size);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool UpdateProcThreadAttribute(
            IntPtr attributeList,
            uint flags,
            IntPtr attribute,
            IntPtr value,
            UIntPtr size,
            IntPtr previousValue,
            IntPtr returnSize);

        [DllImport("kernel32.dll")]
        private static extern void DeleteProcThreadAttributeList(IntPtr attributeList);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint ResumeThread(IntPtr thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool TerminateProcess(IntPtr process, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CloseHandle(IntPtr handle);

        private readonly object sync = new object();
        private IntPtr jobHandle;
        private IntPtr processHandle;
        private bool disposed;

        public int ProcessId { get; private set; }

        private JobProcess(IntPtr job, IntPtr process, int processId) {
            jobHandle = job;
            processHandle = process;
            ProcessId = processId;
        }

        public static JobProcess Start(
            string applicationPath,
            string workingDirectory,
            IDictionary environmentOverrides,
            string standardOutputPath,
            string standardErrorPath) {
            if (String.IsNullOrWhiteSpace(applicationPath) || !Path.IsPathRooted(applicationPath)) {
                throw new ArgumentException("Application path must be absolute.", "applicationPath");
            }
            if (!File.Exists(applicationPath)) {
                throw new FileNotFoundException("Application executable does not exist.", applicationPath);
            }
            if (applicationPath.IndexOf('"') >= 0) {
                throw new ArgumentException("Application path must not contain a quote.", "applicationPath");
            }
            if (String.IsNullOrWhiteSpace(workingDirectory) || !Path.IsPathRooted(workingDirectory) ||
                !Directory.Exists(workingDirectory)) {
                throw new ArgumentException("Working directory must be an existing absolute directory.", "workingDirectory");
            }
            if (environmentOverrides == null) {
                throw new ArgumentNullException("environmentOverrides");
            }
            standardOutputPath = ValidateOutputPath(standardOutputPath, "standardOutputPath");
            standardErrorPath = ValidateOutputPath(standardErrorPath, "standardErrorPath");
            if (String.Equals(
                    standardOutputPath,
                    standardErrorPath,
                    StringComparison.OrdinalIgnoreCase)) {
                throw new ArgumentException("Standard output and error paths must be distinct.");
            }

            IntPtr job = IntPtr.Zero;
            IntPtr process = IntPtr.Zero;
            IntPtr thread = IntPtr.Zero;
            IntPtr environment = IntPtr.Zero;
            IntPtr standardInput = IntPtr.Zero;
            IntPtr standardOutput = IntPtr.Zero;
            IntPtr standardError = IntPtr.Zero;
            IntPtr handleList = IntPtr.Zero;
            IntPtr attributeList = IntPtr.Zero;
            bool attributeListInitialized = false;
            bool assigned = false;

            try {
                job = CreateJobObjectW(IntPtr.Zero, null);
                if (job == IntPtr.Zero) {
                    throw LastError("CreateJobObjectW failed");
                }
                ConfigureKillOnClose(job);

                environment = BuildEnvironmentBlock(environmentOverrides);
                SECURITY_ATTRIBUTES inheritable = new SECURITY_ATTRIBUTES();
                inheritable.nLength = checked((uint) Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES)));
                inheritable.bInheritHandle = true;

                standardInput = CreateFileW(
                    "NUL",
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ref inheritable,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    IntPtr.Zero);
                if (standardInput == INVALID_HANDLE_VALUE) {
                    standardInput = IntPtr.Zero;
                    throw LastError("Opening NUL for child standard input failed");
                }
                standardOutput = CreateFileW(
                    standardOutputPath,
                    GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    ref inheritable,
                    CREATE_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    IntPtr.Zero);
                if (standardOutput == INVALID_HANDLE_VALUE) {
                    standardOutput = IntPtr.Zero;
                    throw LastError("Opening child standard-output log failed");
                }
                standardError = CreateFileW(
                    standardErrorPath,
                    GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    ref inheritable,
                    CREATE_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    IntPtr.Zero);
                if (standardError == INVALID_HANDLE_VALUE) {
                    standardError = IntPtr.Zero;
                    throw LastError("Opening child standard-error log failed");
                }

                handleList = Marshal.AllocHGlobal(checked(IntPtr.Size * 3));
                Marshal.WriteIntPtr(handleList, 0, standardInput);
                Marshal.WriteIntPtr(handleList, IntPtr.Size, standardOutput);
                Marshal.WriteIntPtr(handleList, IntPtr.Size * 2, standardError);

                UIntPtr attributeListSize = UIntPtr.Zero;
                InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref attributeListSize);
                int attributeListProbeError = Marshal.GetLastWin32Error();
                if (attributeListSize == UIntPtr.Zero || attributeListProbeError != ERROR_INSUFFICIENT_BUFFER) {
                    throw new Win32Exception(
                        attributeListProbeError,
                        "InitializeProcThreadAttributeList(size query) failed");
                }
                attributeList = Marshal.AllocHGlobal(checked((int) attributeListSize.ToUInt64()));
                if (!InitializeProcThreadAttributeList(attributeList, 1, 0, ref attributeListSize)) {
                    throw LastError("InitializeProcThreadAttributeList failed");
                }
                attributeListInitialized = true;
                if (!UpdateProcThreadAttribute(
                        attributeList,
                        0,
                        PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                        handleList,
                        new UIntPtr(checked((uint) (IntPtr.Size * 3))),
                        IntPtr.Zero,
                        IntPtr.Zero)) {
                    throw LastError("UpdateProcThreadAttribute(HANDLE_LIST) failed");
                }

                STARTUPINFOEX startup = new STARTUPINFOEX();
                startup.StartupInfo.cb = checked((uint) Marshal.SizeOf(typeof(STARTUPINFOEX)));
                startup.StartupInfo.dwFlags = STARTF_USESHOWWINDOW | STARTF_USESTDHANDLES;
                startup.StartupInfo.wShowWindow = SW_HIDE;
                startup.StartupInfo.hStdInput = standardInput;
                startup.StartupInfo.hStdOutput = standardOutput;
                startup.StartupInfo.hStdError = standardError;
                startup.lpAttributeList = attributeList;

                PROCESS_INFORMATION information;
                StringBuilder commandLine = new StringBuilder("\"" + applicationPath + "\"");
                bool created = CreateProcessW(
                    applicationPath,
                    commandLine,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    true,
                    CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                    environment,
                    workingDirectory,
                    ref startup,
                    out information);
                if (!created) {
                    throw LastError("CreateProcessW(CREATE_SUSPENDED) failed");
                }
                process = information.hProcess;
                thread = information.hThread;

                if (!AssignProcessToJobObject(job, process)) {
                    throw LastError("AssignProcessToJobObject failed before the process was resumed");
                }
                assigned = true;

                if (ResumeThread(thread) == UInt32.MaxValue) {
                    throw LastError("ResumeThread failed after assigning the process to the job");
                }

                CloseHandle(thread);
                thread = IntPtr.Zero;
                JobProcess result = new JobProcess(job, process, checked((int) information.dwProcessId));
                job = IntPtr.Zero;
                process = IntPtr.Zero;
                return result;
            }
            catch {
                if (assigned && job != IntPtr.Zero) {
                    TerminateJobObject(job, 1);
                }
                else if (process != IntPtr.Zero) {
                    TerminateProcess(process, 1);
                }
                if (process != IntPtr.Zero) {
                    WaitForSingleObject(process, 5000);
                }
                throw;
            }
            finally {
                if (attributeListInitialized && attributeList != IntPtr.Zero) {
                    DeleteProcThreadAttributeList(attributeList);
                }
                if (attributeList != IntPtr.Zero) {
                    Marshal.FreeHGlobal(attributeList);
                }
                if (handleList != IntPtr.Zero) {
                    Marshal.FreeHGlobal(handleList);
                }
                if (standardError != IntPtr.Zero) {
                    CloseHandle(standardError);
                }
                if (standardOutput != IntPtr.Zero) {
                    CloseHandle(standardOutput);
                }
                if (standardInput != IntPtr.Zero) {
                    CloseHandle(standardInput);
                }
                if (environment != IntPtr.Zero) {
                    Marshal.FreeHGlobal(environment);
                }
                if (thread != IntPtr.Zero) {
                    CloseHandle(thread);
                }
                if (process != IntPtr.Zero) {
                    CloseHandle(process);
                }
                if (job != IntPtr.Zero) {
                    CloseHandle(job);
                }
            }
        }

        public bool RootHasExited() {
            lock (sync) {
                ThrowIfDisposed();
                uint exitCode;
                if (!GetExitCodeProcess(processHandle, out exitCode)) {
                    throw LastError("GetExitCodeProcess failed");
                }
                return exitCode != STILL_ACTIVE;
            }
        }

        public uint GetRootExitCode() {
            lock (sync) {
                ThrowIfDisposed();
                uint exitCode;
                if (!GetExitCodeProcess(processHandle, out exitCode)) {
                    throw LastError("GetExitCodeProcess failed");
                }
                if (exitCode == STILL_ACTIVE) {
                    throw new InvalidOperationException("Root process is still active.");
                }
                return exitCode;
            }
        }

        public bool WaitForRootExit(int timeoutMilliseconds) {
            if (timeoutMilliseconds < 0) {
                throw new ArgumentOutOfRangeException("timeoutMilliseconds");
            }
            lock (sync) {
                ThrowIfDisposed();
                uint result = WaitForSingleObject(processHandle, checked((uint) timeoutMilliseconds));
                if (result == WAIT_OBJECT_0) {
                    return true;
                }
                if (result == WAIT_TIMEOUT) {
                    return false;
                }
                if (result == WAIT_FAILED) {
                    throw LastError("WaitForSingleObject(root process) failed");
                }
                throw new InvalidOperationException("WaitForSingleObject returned an unexpected value: " + result);
            }
        }

        public int[] GetActiveProcessIds() {
            lock (sync) {
                ThrowIfDisposed();
                JobAccounting accounting = GetAccountingUnsafe();
                int capacity = Math.Max(16, checked((int) accounting.ActiveProcesses + 8));
                for (int attempt = 0; attempt < 8; attempt++) {
                    int bufferSize = checked(8 + (IntPtr.Size * capacity));
                    IntPtr buffer = Marshal.AllocHGlobal(bufferSize);
                    try {
                        for (int offset = 0; offset < bufferSize; offset += 4) {
                            Marshal.WriteInt32(buffer, offset, 0);
                        }
                        uint returned;
                        if (QueryInformationJobObject(
                                jobHandle,
                                JobObjectBasicProcessIdList,
                                buffer,
                                checked((uint) bufferSize),
                                out returned)) {
                            uint count = checked((uint) Marshal.ReadInt32(buffer, 4));
                            if (count > capacity) {
                                capacity = checked((int) count + 8);
                                continue;
                            }
                            int[] processIds = new int[count];
                            for (int index = 0; index < count; index++) {
                                IntPtr raw = Marshal.ReadIntPtr(buffer, 8 + (index * IntPtr.Size));
                                processIds[index] = checked((int) raw.ToInt64());
                            }
                            return processIds;
                        }

                        int error = Marshal.GetLastWin32Error();
                        if (error != ERROR_MORE_DATA && error != ERROR_INSUFFICIENT_BUFFER) {
                            throw new Win32Exception(error, "QueryInformationJobObject(process IDs) failed");
                        }
                        uint assigned = checked((uint) Marshal.ReadInt32(buffer, 0));
                        capacity = Math.Max(capacity * 2, checked((int) assigned + 8));
                    }
                    finally {
                        Marshal.FreeHGlobal(buffer);
                    }
                }
                throw new InvalidOperationException("Job process list did not stabilize after repeated queries.");
            }
        }

        public JobAccounting GetAccounting() {
            lock (sync) {
                ThrowIfDisposed();
                return GetAccountingUnsafe();
            }
        }

        public void Terminate(uint exitCode) {
            lock (sync) {
                ThrowIfDisposed();
                if (!TerminateJobObject(jobHandle, exitCode)) {
                    throw LastError("TerminateJobObject failed");
                }
            }
        }

        private JobAccounting GetAccountingUnsafe() {
            int size = Marshal.SizeOf(typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION));
            IntPtr buffer = Marshal.AllocHGlobal(size);
            try {
                uint returned;
                if (!QueryInformationJobObject(
                        jobHandle,
                        JobObjectBasicAccountingInformation,
                        buffer,
                        checked((uint) size),
                        out returned)) {
                    throw LastError("QueryInformationJobObject(accounting) failed");
                }
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting =
                    (JOBOBJECT_BASIC_ACCOUNTING_INFORMATION) Marshal.PtrToStructure(
                        buffer,
                        typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION));
                return new JobAccounting(
                    accounting.TotalProcesses,
                    accounting.ActiveProcesses,
                    accounting.TotalTerminatedProcesses);
            }
            finally {
                Marshal.FreeHGlobal(buffer);
            }
        }

        private static void ConfigureKillOnClose(IntPtr job) {
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits =
                new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            int size = Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION));
            IntPtr buffer = Marshal.AllocHGlobal(size);
            try {
                Marshal.StructureToPtr(limits, buffer, false);
                if (!SetInformationJobObject(
                        job,
                        JobObjectExtendedLimitInformation,
                        buffer,
                        checked((uint) size))) {
                    throw LastError("SetInformationJobObject(KILL_ON_JOB_CLOSE) failed");
                }
            }
            finally {
                Marshal.FreeHGlobal(buffer);
            }
        }

        private static string ValidateOutputPath(string path, string parameterName) {
            if (String.IsNullOrWhiteSpace(path) || !Path.IsPathRooted(path)) {
                throw new ArgumentException("Process-output path must be absolute.", parameterName);
            }
            string fullPath = Path.GetFullPath(path);
            string parent = Path.GetDirectoryName(fullPath);
            if (String.IsNullOrWhiteSpace(parent) || !Directory.Exists(parent)) {
                throw new ArgumentException(
                    "Process-output parent must be an existing directory.",
                    parameterName);
            }
            if (Directory.Exists(fullPath)) {
                throw new ArgumentException("Process-output path must not be a directory.", parameterName);
            }
            return fullPath;
        }

        private static IntPtr BuildEnvironmentBlock(IDictionary overrides) {
            SortedDictionary<string, string> environment =
                new SortedDictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            foreach (DictionaryEntry entry in Environment.GetEnvironmentVariables()) {
                string key = Convert.ToString(entry.Key);
                if (key.StartsWith("=", StringComparison.Ordinal)) {
                    continue;
                }
                SetEnvironmentValue(environment, key, Convert.ToString(entry.Value));
            }
            foreach (DictionaryEntry entry in overrides) {
                SetEnvironmentValue(
                    environment,
                    Convert.ToString(entry.Key),
                    Convert.ToString(entry.Value));
            }

            StringBuilder block = new StringBuilder();
            foreach (KeyValuePair<string, string> entry in environment) {
                block.Append(entry.Key);
                block.Append('=');
                block.Append(entry.Value);
                block.Append('\0');
            }
            // StringToHGlobalUni appends its own terminator, so the last entry terminator plus
            // that terminator form the exact double-NUL environment-block suffix.
            return Marshal.StringToHGlobalUni(block.ToString());
        }

        private static void SetEnvironmentValue(
            SortedDictionary<string, string> environment,
            string key,
            string value) {
            if (String.IsNullOrEmpty(key) || key.IndexOf('=') >= 0 || key.IndexOf('\0') >= 0) {
                throw new ArgumentException("Environment variable name is invalid.");
            }
            if (value == null || value.IndexOf('\0') >= 0) {
                throw new ArgumentException("Environment variable value is invalid for '" + key + "'.");
            }
            environment[key] = value;
        }

        private void ThrowIfDisposed() {
            if (disposed) {
                throw new ObjectDisposedException("JobProcess");
            }
        }

        private static Win32Exception LastError(string message) {
            return new Win32Exception(Marshal.GetLastWin32Error(), message);
        }

        public void Dispose() {
            Dispose(true);
            GC.SuppressFinalize(this);
        }

        private void Dispose(bool disposing) {
            IntPtr job;
            IntPtr process;
            lock (sync) {
                if (disposed) {
                    return;
                }
                disposed = true;
                job = jobHandle;
                process = processHandle;
                jobHandle = IntPtr.Zero;
                processHandle = IntPtr.Zero;
            }
            if (job != IntPtr.Zero) {
                CloseHandle(job);
            }
            if (process != IntPtr.Zero) {
                CloseHandle(process);
            }
        }

        ~JobProcess() {
            Dispose(false);
        }
    }
}
'@
}

function Get-ApplicationWindows {
    param([Parameter(Mandatory = $true)] [int] $ProcessId)

    Add-WindowInterop
    return @([VhmReleaseSmoke.NativeWindows]::ForProcess($ProcessId))
}

function Get-ProcessIdsByName {
    param([Parameter(Mandatory = $true)] [string] $ProcessName)

    $processes = @()
    try {
        $processes = @([System.Diagnostics.Process]::GetProcessesByName($ProcessName))
        return @($processes | ForEach-Object { [int] $_.Id } | Sort-Object)
    }
    finally {
        foreach ($process in $processes) {
            $process.Dispose()
        }
    }
}

function ConvertTo-WindowReport {
    param([Parameter(Mandatory = $true)] [object] $Window)

    $handleWidth = [IntPtr]::Size * 2
    return [ordered]@{
        handle = ('0x{0:X' + $handleWidth + '}') -f [uint64] $Window.Handle.ToInt64()
        processId = [uint32] $Window.ProcessId
        threadId = [uint32] $Window.ThreadId
        visible = [bool] $Window.Visible
        minimized = [bool] $Window.Minimized
        ownerHandle = ('0x{0:X' + $handleWidth + '}') -f [uint64] $Window.OwnerHandle.ToInt64()
        title = [string] $Window.Title
        className = [string] $Window.ClassName
    }
}

function ConvertTo-WindowInventoryReport {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [object[]] $Windows
    )

    return @($Windows | ForEach-Object { ConvertTo-WindowReport -Window $_ })
}

function Select-MainApplicationWindow {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [object[]] $Windows,
        [Parameter(Mandatory = $true)] [int] $ProcessId
    )

    # tauri-plugin-single-instance owns a visible, unowned message-only coordination window in
    # the root process. It is not an application surface. Select only real Tauri webview windows
    # before applying the one-visible-main-window invariant.
    $eligible = @($Windows | Where-Object { $_.ClassName -ceq 'Tauri Window' })
    $visible = @($eligible | Where-Object { $_.Visible })
    if ($visible.Count -eq 0) {
        return $null
    }
    $unowned = @($visible | Where-Object { $_.OwnerHandle -eq [IntPtr]::Zero })
    $candidates = if ($unowned.Count -gt 0) { $unowned } else { $visible }
    $titled = @($candidates | Where-Object { -not [string]::IsNullOrWhiteSpace($_.Title) })
    if ($titled.Count -gt 0) {
        $candidates = $titled
    }
    if ($candidates.Count -ne 1) {
        $inventory = ConvertTo-WindowInventoryReport -Windows $Windows
        throw "Expected exactly one visible main window for root PID $ProcessId; found $($candidates.Count) candidates. Inventory: $($inventory | ConvertTo-Json -Depth 5 -Compress)"
    }
    return $candidates[0]
}

function Get-ProcessLogCapture {
    param(
        [Parameter(Mandatory = $true)] [string] $LiteralPath,
        [Parameter(Mandatory = $true)] [string] $EnvironmentRoot,
        [Parameter(Mandatory = $true)] [string] $StreamName,
        [ValidateRange(1024, 1048576)] [int] $MaximumBytes = 65536
    )

    $rootCanonical = Get-CanonicalExistingPath `
        -LiteralPath $EnvironmentRoot `
        -RequireDirectory $true `
        -Description 'release smoke environment root for process-output capture'
    $fullPath = [System.IO.Path]::GetFullPath($LiteralPath)
    if (-not (Test-PathWithinRoot -Root $rootCanonical -Candidate $fullPath)) {
        throw "$StreamName log path escapes the release smoke environment root: '$fullPath'."
    }
    if (-not (Test-Path -LiteralPath $fullPath)) {
        return [ordered]@{
            stream = $StreamName
            path = $fullPath
            exists = $false
            byteLength = 0L
            capturedByteLength = 0
            truncated = $false
            text = ''
        }
    }

    $resolvedLogs = @(Resolve-Path -LiteralPath $fullPath -ErrorAction Stop)
    if ($resolvedLogs.Count -ne 1) {
        throw "$StreamName process-output log must resolve to exactly one path: '$fullPath'."
    }
    $canonical = [System.IO.Path]::GetFullPath($resolvedLogs[0].ProviderPath)
    if (-not (Test-PathWithinRoot -Root $rootCanonical -Candidate $canonical)) {
        throw "$StreamName log resolved outside the release smoke environment root: '$canonical'."
    }
    $item = Get-Item -LiteralPath $canonical -Force -ErrorAction Stop
    if ($item.PSIsContainer) {
        throw "$StreamName process-output log must be a regular file: '$canonical'."
    }
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$StreamName process-output log must not be a reparse point: '$canonical'."
    }

    $byteLength = [long] $item.Length
    $captureLength = [int] [System.Math]::Min($byteLength, [long] $MaximumBytes)
    $bytes = [byte[]]::new($captureLength)
    $bytesRead = 0
    if ($captureLength -gt 0) {
        $share = [System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete
        $stream = [System.IO.FileStream]::new(
            $canonical,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            $share
        )
        try {
            if ($byteLength -gt $captureLength) {
                [void] $stream.Seek($byteLength - $captureLength, [System.IO.SeekOrigin]::Begin)
            }
            while ($bytesRead -lt $captureLength) {
                $count = $stream.Read($bytes, $bytesRead, $captureLength - $bytesRead)
                if ($count -eq 0) {
                    break
                }
                $bytesRead += $count
            }
        }
        finally {
            $stream.Dispose()
        }
    }

    $encoding = [System.Text.UTF8Encoding]::new($false, $false)
    return [ordered]@{
        stream = $StreamName
        path = $canonical
        exists = $true
        byteLength = $byteLength
        capturedByteLength = $bytesRead
        truncated = ($byteLength -gt $bytesRead)
        text = if ($bytesRead -gt 0) { $encoding.GetString($bytes, 0, $bytesRead) } else { '' }
    }
}

function Get-ProcessOutputCapture {
    param(
        [Parameter(Mandatory = $true)] [string] $StandardOutputPath,
        [Parameter(Mandatory = $true)] [string] $StandardErrorPath,
        [Parameter(Mandatory = $true)] [string] $EnvironmentRoot,
        [ValidateRange(1024, 1048576)] [int] $MaximumBytesPerStream = 65536
    )

    return [ordered]@{
        maximumBytesPerStream = $MaximumBytesPerStream
        stdout = Get-ProcessLogCapture `
            -LiteralPath $StandardOutputPath `
            -EnvironmentRoot $EnvironmentRoot `
            -StreamName 'stdout' `
            -MaximumBytes $MaximumBytesPerStream
        stderr = Get-ProcessLogCapture `
            -LiteralPath $StandardErrorPath `
            -EnvironmentRoot $EnvironmentRoot `
            -StreamName 'stderr' `
            -MaximumBytes $MaximumBytesPerStream
    }
}

function Format-ProcessOutputEvidence {
    param([Parameter(Mandatory = $true)] [object] $Capture)

    $parts = [System.Collections.Generic.List[string]]::new()
    foreach ($streamName in @('stderr', 'stdout')) {
        $stream = $Capture[$streamName]
        if (-not $stream.exists) {
            $parts.Add("$streamName log was not created")
            continue
        }
        if ($stream.byteLength -eq 0) {
            $parts.Add("$streamName log is empty")
            continue
        }
        $scope = if ($stream.truncated) { 'tail' } else { 'complete' }
        $parts.Add(
            "$streamName $scope capture ($($stream.capturedByteLength)/$($stream.byteLength) bytes):`n$($stream.text.TrimEnd())"
        )
    }
    return [string]::Join("`n", $parts)
}

function Get-RealProductPaths {
    param([Parameter(Mandatory = $true)] [string] $Identifier)

    if ($Identifier.IndexOfAny([System.IO.Path]::GetInvalidFileNameChars()) -ge 0 -or
        $Identifier -in @('.', '..') -or
        $Identifier.Contains([System.IO.Path]::DirectorySeparatorChar) -or
        $Identifier.Contains([System.IO.Path]::AltDirectorySeparatorChar)) {
        throw "AppIdentifier must be a single valid path segment; got '$Identifier'."
    }

    Add-KnownFolderInterop
    $roaming = [VhmReleaseSmoke.KnownFolders]::GetPath('3EB685DB-65F9-4CF6-A03A-E3EF65729F3D')
    $local = [VhmReleaseSmoke.KnownFolders]::GetPath('F1B32785-6FBA-4FCF-9D55-7B8E7F157091')
    return [ordered]@{
        data = [System.IO.Path]::GetFullPath((Join-Path $roaming $Identifier))
        cache = [System.IO.Path]::GetFullPath((Join-Path $local $Identifier))
    }
}

function Get-TreeFingerprint {
    param(
        [Parameter(Mandatory = $true)] [string] $Root,
        [Parameter(Mandatory = $true)] [string] $Description
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    if (-not (Test-Path -LiteralPath $rootFull)) {
        $missingBytes = [System.Text.Encoding]::UTF8.GetBytes('<missing>')
        return [pscustomobject]@{
            exists = $false
            entryCount = 0
            totalFileBytes = 0L
            digest = [System.Convert]::ToHexString(
                [System.Security.Cryptography.SHA256]::HashData($missingBytes)
            ).ToLowerInvariant()
        }
    }

    $rootItem = Get-Item -LiteralPath $rootFull -Force -ErrorAction Stop
    if (-not $rootItem.PSIsContainer) {
        throw "$Description is not a directory: '$rootFull'."
    }
    if (($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Description root is a reparse point; unchanged-state proof is not safe: '$rootFull'."
    }

    $records = [System.Collections.Generic.List[string]]::new()
    $records.Add("D|.|$([long] $rootItem.Attributes)|$($rootItem.CreationTimeUtc.Ticks)|$($rootItem.LastWriteTimeUtc.Ticks)")
    $pending = [System.Collections.Generic.Stack[System.IO.DirectoryInfo]]::new()
    $pending.Push([System.IO.DirectoryInfo] $rootItem)
    $totalBytes = 0L

    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($item in $directory.EnumerateFileSystemInfos()) {
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Description contains a reparse point; unchanged-state proof is not safe: '$($item.FullName)'."
            }

            $relative = [System.IO.Path]::GetRelativePath($rootFull, $item.FullName).Replace('\', '/')
            if ($item -is [System.IO.DirectoryInfo]) {
                $records.Add("D|$relative|$([long] $item.Attributes)|$($item.CreationTimeUtc.Ticks)|$($item.LastWriteTimeUtc.Ticks)")
                $pending.Push($item)
            }
            elseif ($item -is [System.IO.FileInfo]) {
                $hash = Get-Sha256 -LiteralPath $item.FullName
                $records.Add("F|$relative|$([long] $item.Attributes)|$($item.Length)|$($item.CreationTimeUtc.Ticks)|$($item.LastWriteTimeUtc.Ticks)|$hash")
                $totalBytes += $item.Length
            }
            else {
                throw "$Description contains an unsupported filesystem entry: '$($item.FullName)'."
            }
        }
    }

    $orderedRecords = @($records.ToArray() | Sort-Object -CaseSensitive)
    $payload = [System.Text.Encoding]::UTF8.GetBytes([string]::Join("`n", $orderedRecords))
    return [pscustomobject]@{
        exists = $true
        entryCount = $orderedRecords.Count
        totalFileBytes = $totalBytes
        digest = [System.Convert]::ToHexString(
            [System.Security.Cryptography.SHA256]::HashData($payload)
        ).ToLowerInvariant()
    }
}

function Test-FingerprintEqual {
    param(
        [Parameter(Mandatory = $true)] [object] $Before,
        [Parameter(Mandatory = $true)] [object] $After
    )

    return $Before.exists -eq $After.exists -and
        $Before.entryCount -eq $After.entryCount -and
        $Before.totalFileBytes -eq $After.totalFileBytes -and
        [string]::Equals($Before.digest, $After.digest, [System.StringComparison]::Ordinal)
}

function Update-ObservedJobProcesses {
    param(
        [Parameter(Mandatory = $true)] [object] $Job,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [System.Collections.Generic.HashSet[int]] $Observed
    )

    $activeProcessIds = @($Job.GetActiveProcessIds())
    foreach ($processId in $activeProcessIds) {
        [void] $Observed.Add([int] $processId)
    }
    return $activeProcessIds
}

function Assert-NoActiveJobProcesses {
    param(
        [Parameter(Mandatory = $true)] [object] $Job,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [System.Collections.Generic.HashSet[int]] $Observed,
        [Parameter(Mandatory = $true)] [datetime] $Deadline
    )

    do {
        $activeProcessIds = @(Update-ObservedJobProcesses -Job $Job -Observed $Observed)
        if ($activeProcessIds.Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 200
    } while ([datetime]::UtcNow -lt $Deadline)

    throw "Windows Job still contains active process IDs after shutdown: $([string]::Join(', ', $activeProcessIds))."
}

function Format-FingerprintSummary {
    param([Parameter(Mandatory = $true)] [object] $Fingerprint)

    return "exists=$($Fingerprint.exists), entryCount=$($Fingerprint.entryCount), totalFileBytes=$($Fingerprint.totalFileBytes), digest=$($Fingerprint.digest)"
}

function ConvertTo-FingerprintReport {
    param([Parameter(Mandatory = $true)] [object] $Fingerprint)

    return [ordered]@{
        exists = [bool] $Fingerprint.exists
        entryCount = [int] $Fingerprint.entryCount
        totalFileBytes = [long] $Fingerprint.totalFileBytes
        digest = [string] $Fingerprint.digest
    }
}

function Assert-IsolationRoot {
    param(
        [Parameter(Mandatory = $true)] [string] $Root,
        [Parameter(Mandatory = $true)] [string] $Parent,
        [Parameter(Mandatory = $true)] [string[]] $ProtectedPaths,
        [switch] $RequireFresh,
        [switch] $InspectAllChildren
    )

    $rootCanonical = Get-CanonicalExistingPath -LiteralPath $Root -RequireDirectory $true -Description 'release smoke root'
    $parentCanonical = Get-CanonicalExistingPath -LiteralPath $Parent -RequireDirectory $true -Description 'release smoke isolation parent'
    $rootItem = Get-Item -LiteralPath $rootCanonical -Force
    $parentItem = Get-Item -LiteralPath $parentCanonical -Force
    if (($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
        ($parentItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'The isolation parent and smoke root must not be reparse points.'
    }
    if (-not $rootItem.Name.StartsWith($SmokeRootPrefix, [System.StringComparison]::Ordinal)) {
        throw "Release smoke root must start with '$SmokeRootPrefix': '$rootCanonical'."
    }
    if (-not [string]::Equals(
            $rootItem.Parent.FullName.TrimEnd('\', '/'),
            $parentCanonical.TrimEnd('\', '/'),
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw "Release smoke root must be a direct child of '$parentCanonical'."
    }

    $markerPath = Join-Path $rootCanonical $SmokeMarkerName
    $markerItem = Get-Item -LiteralPath $markerPath -Force -ErrorAction Stop
    if ($markerItem.PSIsContainer -or
        ($markerItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
        [System.IO.File]::ReadAllText($markerPath) -cne $SmokeMarkerContent) {
        throw "Release smoke marker is missing, linked, or has unexpected content: '$markerPath'."
    }
    if ($RequireFresh) {
        $entries = @(Get-ChildItem -LiteralPath $rootCanonical -Force -ErrorAction Stop)
        if ($entries.Count -ne 1 -or
            -not [string]::Equals($entries[0].Name, $SmokeMarkerName, [System.StringComparison]::Ordinal)) {
            throw "Release smoke root must be fresh and contain only '$SmokeMarkerName' before launch."
        }
    }

    foreach ($protectedPath in $ProtectedPaths) {
        if (Test-PathsOverlap -First $rootCanonical -Second $protectedPath) {
            throw "Release smoke root overlaps protected real application path '$protectedPath'."
        }
    }

    foreach ($childName in @('data', 'cache')) {
        $childPath = [System.IO.Path]::GetFullPath((Join-Path $rootCanonical $childName))
        if (-not (Test-PathWithinRoot -Root $rootCanonical -Candidate $childPath)) {
            throw "Release smoke '$childName' path escapes the smoke root."
        }
        if (Test-Path -LiteralPath $childPath) {
            $childItem = Get-Item -LiteralPath $childPath -Force
            if (-not $childItem.PSIsContainer -or
                ($childItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Release smoke '$childName' path is not a real directory: '$childPath'."
            }
        }
    }

    if ($InspectAllChildren) {
        $pending = [System.Collections.Generic.Stack[System.IO.DirectoryInfo]]::new()
        $pending.Push([System.IO.DirectoryInfo] $rootItem)
        while ($pending.Count -gt 0) {
            $directory = $pending.Pop()
            foreach ($item in $directory.EnumerateFileSystemInfos()) {
                if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                    throw "Release smoke cleanup refused a reparse point: '$($item.FullName)'."
                }
                if ($item -is [System.IO.DirectoryInfo]) {
                    $pending.Push($item)
                }
            }
        }
    }

    return $rootCanonical
}

function Assert-EnvironmentRoot {
    param(
        [Parameter(Mandatory = $true)] [string] $Root,
        [Parameter(Mandatory = $true)] [string] $Parent,
        [Parameter(Mandatory = $true)] [string[]] $ProtectedPaths,
        [switch] $InspectAllChildren
    )

    $rootCanonical = Get-CanonicalExistingPath -LiteralPath $Root -RequireDirectory $true -Description 'release smoke environment root'
    $parentCanonical = Get-CanonicalExistingPath -LiteralPath $Parent -RequireDirectory $true -Description 'release smoke isolation parent'
    $rootItem = Get-Item -LiteralPath $rootCanonical -Force
    $parentItem = Get-Item -LiteralPath $parentCanonical -Force
    if (($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
        ($parentItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'The isolation parent and smoke environment root must not be reparse points.'
    }
    if (-not $rootItem.Name.StartsWith($EnvironmentRootPrefix, [System.StringComparison]::Ordinal)) {
        throw "Release smoke environment root must start with '$EnvironmentRootPrefix': '$rootCanonical'."
    }
    if (-not [string]::Equals(
            $rootItem.Parent.FullName.TrimEnd('\', '/'),
            $parentCanonical.TrimEnd('\', '/'),
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw "Release smoke environment root must be a direct child of '$parentCanonical'."
    }

    $markerPath = Join-Path $rootCanonical $EnvironmentMarkerName
    $markerItem = Get-Item -LiteralPath $markerPath -Force -ErrorAction Stop
    if ($markerItem.PSIsContainer -or
        ($markerItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
        [System.IO.File]::ReadAllText($markerPath) -cne $EnvironmentMarkerContent) {
        throw "Release smoke environment marker is missing, linked, or has unexpected content: '$markerPath'."
    }

    foreach ($protectedPath in $ProtectedPaths) {
        if (Test-PathsOverlap -First $rootCanonical -Second $protectedPath) {
            throw "Release smoke environment root overlaps protected path '$protectedPath'."
        }
    }

    if ($InspectAllChildren) {
        $pending = [System.Collections.Generic.Stack[System.IO.DirectoryInfo]]::new()
        $pending.Push([System.IO.DirectoryInfo] $rootItem)
        while ($pending.Count -gt 0) {
            $directory = $pending.Pop()
            foreach ($item in $directory.EnumerateFileSystemInfos()) {
                if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                    throw "Release smoke environment cleanup refused a reparse point: '$($item.FullName)'."
                }
                if ($item -is [System.IO.DirectoryInfo]) {
                    $pending.Push($item)
                }
            }
        }
    }

    return $rootCanonical
}

function Get-PythonExecutable {
    param([AllowEmptyString()] [string] $RequestedPath)

    if (-not [string]::IsNullOrWhiteSpace($RequestedPath)) {
        return Get-CanonicalExistingPath -LiteralPath $RequestedPath -RequireDirectory $false -Description 'Python executable'
    }

    $command = Get-Command python.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1
    return Get-CanonicalExistingPath -LiteralPath $command.Source -RequireDirectory $false -Description 'Python executable'
}

function Invoke-SqliteInspection {
    param(
        [Parameter(Mandatory = $true)] [string] $PythonPath,
        [Parameter(Mandatory = $true)] [string] $DatabasePath,
        [Parameter(Mandatory = $true)] [int] $TimeoutSeconds,
        [Parameter(Mandatory = $true)] [string] $IsolatedTemp
    )

    $pythonCode = @'
import json
import sqlite3
import sys
from pathlib import Path

def compare_values(left, right):
    return (left > right) - (left < right)

def is_ascii_digit(character):
    return "0" <= character <= "9"

RUST_UNICODE_17_LOWER_OVERRIDES = {
    "\u1c89": "\u1c8a",
    "\ua7cb": "\u0264",
    "\ua7cc": "\ua7cd",
    "\ua7ce": "\ua7cf",
    "\ua7d2": "\ua7d3",
    "\ua7d4": "\ua7d5",
    "\ua7da": "\ua7db",
    "\ua7dc": "\u019b",
}

def rust_lower(value):
    # Python 3.13 ships Unicode 15.1 tables while the release Rust toolchain uses Unicode 17.
    # Apply the Unicode 16/17 casing additions after Python handles full/contextual lowercasing.
    lowered = value.lower()
    result = []
    for character in lowered:
        codepoint = ord(character)
        if 0x10D50 <= codepoint <= 0x10D65:
            result.append(chr(codepoint + 0x20))
        elif 0x16EA0 <= codepoint <= 0x16EB8:
            result.append(chr(codepoint + 0x1B))
        else:
            result.append(RUST_UNICODE_17_LOWER_OVERRIDES.get(character, character))
    return "".join(result)

def next_natural_name_chunk(value, offset):
    if offset >= len(value):
        return None, offset
    starts_with_digit = is_ascii_digit(value[offset])
    end = offset
    while end < len(value) and is_ascii_digit(value[end]) == starts_with_digit:
        end += 1
    return (value[offset:end], starts_with_digit), end

def compare_numeric_name_chunks(left, right):
    left_significant = left.lstrip("0") or "0"
    right_significant = right.lstrip("0") or "0"
    length_ordering = compare_values(len(left_significant), len(right_significant))
    if length_ordering != 0:
        return length_ordering
    return compare_values(left_significant, right_significant)

def compare_clip_names(left, right):
    left_offset = 0
    right_offset = 0
    while True:
        left_chunk, left_offset = next_natural_name_chunk(left, left_offset)
        right_chunk, right_offset = next_natural_name_chunk(right, right_offset)
        if left_chunk is None and right_chunk is None:
            return 0
        if left_chunk is None:
            return -1
        if right_chunk is None:
            return 1
        if left_chunk[1] and right_chunk[1]:
            ordering = compare_numeric_name_chunks(left_chunk[0], right_chunk[0])
        else:
            ordering = compare_values(rust_lower(left_chunk[0]), rust_lower(right_chunk[0]))
        if ordering != 0:
            return ordering

database_path = Path(sys.argv[1]).resolve()
uri = database_path.as_uri() + "?mode=ro"
connection = sqlite3.connect(uri, uri=True, timeout=5)
try:
    # Keep this callback byte-for-byte equivalent in behavior to db.rs compare_clip_names.
    # PRAGMA quick_check traverses indexes and therefore requires every indexed collation.
    connection.create_collation("VHM_CLIP_NAME", compare_clip_names)
    connection.execute("PRAGMA query_only = ON")
    schema_version = connection.execute("PRAGMA user_version").fetchone()[0]
    quick_check = connection.execute("PRAGMA quick_check").fetchone()[0]
    tables = sorted(
        row[0]
        for row in connection.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
        )
    )
    columns = {
        table: sorted(row[1] for row in connection.execute(f"PRAGMA table_info({table})"))
        for table in ("clips", "clip_events", "scan_runs")
    }
    tags = [row[0] for row in connection.execute("SELECT name FROM tags ORDER BY name")]
    counts = {
        table: connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
        for table in (
            "scan_runs",
            "source_dirs",
            "clips",
            "clip_trash_snapshots",
            "clip_delete_intents",
        )
    }
finally:
    connection.close()

print(json.dumps({
    "schemaVersion": schema_version,
    "quickCheck": quick_check,
    "tables": tables,
    "columns": columns,
    "tags": tags,
    "counts": counts,
}, ensure_ascii=True))
'@

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $PythonPath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $startInfo.ArgumentList.Add('-I')
    $startInfo.ArgumentList.Add('-c')
    $startInfo.ArgumentList.Add($pythonCode)
    $startInfo.ArgumentList.Add($DatabasePath)
    $startInfo.Environment['PYTHONDONTWRITEBYTECODE'] = '1'
    $startInfo.Environment['PYTHONNOUSERSITE'] = '1'
    $startInfo.Environment['PYTHONIOENCODING'] = 'utf-8'
    $startInfo.Environment['PYTHONUTF8'] = '1'
    $startInfo.Environment['TEMP'] = $IsolatedTemp
    $startInfo.Environment['TMP'] = $IsolatedTemp

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw 'Python SQLite inspector did not start.'
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill($true)
            [void] $process.WaitForExit(5000)
            throw "Python SQLite inspection exceeded $TimeoutSeconds seconds."
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw "Python SQLite inspection failed with exit code $($process.ExitCode): $stderr"
        }
        if ([string]::IsNullOrWhiteSpace($stdout)) {
            throw 'Python SQLite inspection returned no JSON.'
        }
        return $stdout | ConvertFrom-Json -Depth 20
    }
    finally {
        $process.Dispose()
    }
}

function Assert-DatabaseInspection {
    param([Parameter(Mandatory = $true)] [object] $Inspection)

    if ([int] $Inspection.schemaVersion -ne $ExpectedSchemaVersion) {
        throw "Expected database schema v$ExpectedSchemaVersion, got v$($Inspection.schemaVersion)."
    }
    if ($Inspection.quickCheck -cne 'ok') {
        throw "SQLite quick_check did not return 'ok': '$($Inspection.quickCheck)'."
    }

    $actualTables = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::Ordinal
    )
    foreach ($table in @($Inspection.tables)) {
        [void] $actualTables.Add([string] $table)
    }
    $missingTables = @($RequiredTables | Where-Object { -not $actualTables.Contains($_) })
    if ($missingTables.Count -gt 0) {
        throw "Database is missing required tables: $([string]::Join(', ', $missingTables))."
    }

    foreach ($table in $RequiredColumns.Keys) {
        $property = $Inspection.columns.PSObject.Properties[$table]
        if ($null -eq $property) {
            throw "SQLite inspection did not report columns for '$table'."
        }
        $actualColumns = [System.Collections.Generic.HashSet[string]]::new(
            [System.StringComparer]::Ordinal
        )
        foreach ($column in @($property.Value)) {
            [void] $actualColumns.Add([string] $column)
        }
        $missingColumns = @($RequiredColumns[$table] | Where-Object { -not $actualColumns.Contains($_) })
        if ($missingColumns.Count -gt 0) {
            throw "Database table '$table' is missing required columns: $([string]::Join(', ', $missingColumns))."
        }
    }

    if (@($Inspection.tags).Count -ne 0) {
        throw "Fresh database custom tag catalog must be empty; got $(@($Inspection.tags).Count) tags."
    }

    foreach ($table in @(
        'scan_runs',
        'source_dirs',
        'clips',
        'clip_trash_snapshots',
        'clip_delete_intents'
    )) {
        $property = $Inspection.counts.PSObject.Properties[$table]
        if ($null -eq $property) {
            throw "SQLite inspection did not report '$table' count."
        }
        if ([long] $property.Value -ne 0) {
            throw "Startup smoke must not scan or import data: '$table' contains $($property.Value) rows."
        }
    }
}

if (-not (Test-IsWindows)) {
    throw 'windows-release-smoke.ps1 must run on Windows.'
}
if (-not [string]::IsNullOrEmpty($env:WEBVIEW2_USER_DATA_FOLDER)) {
    throw 'Refusing to inherit WEBVIEW2_USER_DATA_FOLDER during a release smoke test.'
}
if (-not [string]::IsNullOrWhiteSpace($env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS)) {
    throw 'Refusing to inherit WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS during a release smoke test.'
}
Add-KnownFolderInterop
Add-WindowInterop
Add-JobProcessInterop

$executable = Get-CanonicalExistingPath -LiteralPath $ExecutablePath -RequireDirectory $false -Description 'application executable'
$resources = Get-CanonicalExistingPath -LiteralPath $ResourceDirectory -RequireDirectory $true -Description 'application resource directory'
$isolationParentCanonical = Get-CanonicalExistingPath -LiteralPath $IsolationParent -RequireDirectory $true -Description 'release smoke isolation parent'
$python = Get-PythonExecutable -RequestedPath $PythonExecutablePath
Assert-PeFile -LiteralPath $executable -Description 'application executable'

$executableHash = Get-Sha256 -LiteralPath $executable
if (-not [string]::IsNullOrWhiteSpace($ExpectedExecutableSha256)) {
    Assert-Sha256Text -Value $ExpectedExecutableSha256 -Description 'expected application executable SHA-256'
    if (-not [string]::Equals($executableHash, $ExpectedExecutableSha256, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Application executable SHA-256 mismatch. Expected '$ExpectedExecutableSha256', got '$executableHash'."
    }
}

$resourcesItem = Get-Item -LiteralPath $resources -Force
$isolationParentItem = Get-Item -LiteralPath $isolationParentCanonical -Force
if (($resourcesItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Application resource directory must not be a reparse point: '$resources'."
}
if (($isolationParentItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Release smoke isolation parent must not be a reparse point: '$isolationParentCanonical'."
}

$realPaths = Get-RealProductPaths -Identifier $AppIdentifier
$protectedPaths = @($realPaths.data, $realPaths.cache)
$executableName = [System.IO.Path]::GetFileName($executable)
$processName = [System.IO.Path]::GetFileNameWithoutExtension($executable)
$conflictingProcessIds = [System.Collections.Generic.List[int]]::new()
$sameNameProcesses = @()
try {
    $sameNameProcesses = @([System.Diagnostics.Process]::GetProcessesByName($processName))
    foreach ($process in $sameNameProcesses) {
        $conflictingProcessIds.Add($process.Id)
    }
}
catch {
    throw "Enumerating existing '$executableName' processes failed; concurrent-state proof is unavailable: $($_.Exception.Message)"
}
finally {
    foreach ($process in $sameNameProcesses) {
        $process.Dispose()
    }
}
if ($conflictingProcessIds.Count -gt 0) {
    throw "Refusing to run while another process with the application executable name exists: ${executableName}[$([string]::Join(', ', $conflictingProcessIds))]."
}

$realDataBefore = Get-TreeFingerprint -Root $realPaths.data -Description 'real application data directory'
$realCacheBefore = Get-TreeFingerprint -Root $realPaths.cache -Description 'real application cache directory'

$smokeRoot = $null
$environmentRoot = $null
$jobProcess = $null
$secondInstanceProcess = $null
$observedJobProcessIds = [System.Collections.Generic.HashSet[int]]::new()
$observedSecondInstanceProcessIds = [System.Collections.Generic.HashSet[int]]::new()
$rootExitCode = $null
$jobAccounting = $null
$realDataAfter = $null
$realCacheAfter = $null
$primaryError = $null
$cleanupError = $null
$report = $null
$realStateChecked = $false
$standardOutputPath = $null
$standardErrorPath = $null
$secondInstanceStandardOutputPath = $null
$secondInstanceStandardErrorPath = $null
$processOutputCapture = $null
$secondInstanceProcessOutputCapture = $null
$appWindowHandle = [IntPtr]::Zero
$appWindowInfo = $null
$startupWindowInventory = @()
$singleInstanceDiagnostics = $null
$closeDiagnostics = $null
$startedAt = [datetime]::UtcNow

try {
    $runId = [guid]::NewGuid().ToString('N')
    $smokeRootCandidate = Join-Path $isolationParentCanonical ($SmokeRootPrefix + $runId)
    [void] (New-Item -ItemType Directory -Path $smokeRootCandidate -ErrorAction Stop)
    $smokeRoot = Get-CanonicalExistingPath -LiteralPath $smokeRootCandidate -RequireDirectory $true -Description 'release smoke root'
    [System.IO.File]::WriteAllText(
        (Join-Path $smokeRoot $SmokeMarkerName),
        $SmokeMarkerContent,
        [System.Text.UTF8Encoding]::new($false)
    )
    [void] (Assert-IsolationRoot -Root $smokeRoot -Parent $isolationParentCanonical -ProtectedPaths $protectedPaths -RequireFresh)

    $environmentRootCandidate = Join-Path $isolationParentCanonical ($EnvironmentRootPrefix + $runId)
    [void] (New-Item -ItemType Directory -Path $environmentRootCandidate -ErrorAction Stop)
    $environmentRoot = Get-CanonicalExistingPath -LiteralPath $environmentRootCandidate -RequireDirectory $true -Description 'release smoke environment root'
    [System.IO.File]::WriteAllText(
        (Join-Path $environmentRoot $EnvironmentMarkerName),
        $EnvironmentMarkerContent,
        [System.Text.UTF8Encoding]::new($false)
    )
    [void] (Assert-EnvironmentRoot `
        -Root $environmentRoot `
        -Parent $isolationParentCanonical `
        -ProtectedPaths @($protectedPaths + $smokeRoot))

    $environmentPaths = [ordered]@{
        temp = Join-Path $environmentRoot 'Temp'
    }
    [void] (New-Item -ItemType Directory -Path $environmentPaths.temp -Force -ErrorAction Stop)
    $standardOutputPath = Join-Path $environmentRoot 'application.stdout.log'
    $standardErrorPath = Join-Path $environmentRoot 'application.stderr.log'
    $secondInstanceStandardOutputPath = Join-Path $environmentRoot 'second-instance.stdout.log'
    $secondInstanceStandardErrorPath = Join-Path $environmentRoot 'second-instance.stderr.log'

    $ffmpegPath = Join-Path $environmentRoot 'missing\ffmpeg.exe'
    if ($FfmpegMode -eq 'ResourceOverride') {
        $ffmpegCandidate = [System.IO.Path]::GetFullPath((Join-Path $resources 'bin\ffmpeg.exe'))
        if (-not (Test-PathWithinRoot -Root $resources -Candidate $ffmpegCandidate)) {
            throw 'FFmpeg resource path escapes the supplied resource directory.'
        }
        $ffmpegPath = Get-CanonicalExistingPath -LiteralPath $ffmpegCandidate -RequireDirectory $false -Description 'bundled FFmpeg executable'
        $ffmpegItem = Get-Item -LiteralPath $ffmpegPath -Force
        if (($ffmpegItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Bundled FFmpeg executable must not be a reparse point: '$ffmpegPath'."
        }
        Assert-PeFile -LiteralPath $ffmpegPath -Description 'bundled FFmpeg executable'
    }

    $databasePath = Join-Path $smokeRoot 'data\highlight-index.sqlite3'
    $thumbnailCachePath = Join-Path $smokeRoot 'cache\thumbnails'
    $webView2Path = Join-Path $smokeRoot 'webview2'

    $childEnvironment = @{
        'VHM_RELEASE_SMOKE_ROOT' = $smokeRoot
        # WebView2 environment/registry overrides take precedence over the userDataFolder argument
        # passed by Wry. Supplying the same marker-gated path here protects pre-setup and secondary
        # launches while the Rust window builder remains an independent, matching constraint.
        'WEBVIEW2_USER_DATA_FOLDER' = $webView2Path
        'TEMP' = $environmentPaths.temp
        'TMP' = $environmentPaths.temp
        'VHM_FFMPEG_PATH' = $ffmpegPath
        'VHM_REAL_SCAN_DB' = (Join-Path $environmentRoot 'prohibited-real-scan.sqlite3')
        'VHM_REAL_SCAN_ROOT' = (Join-Path $environmentRoot 'prohibited-real-scan-root')
        'RUST_BACKTRACE' = '1'
    }

    [void] (Assert-IsolationRoot `
        -Root $smokeRoot `
        -Parent $isolationParentCanonical `
        -ProtectedPaths @($protectedPaths + $environmentRoot) `
        -RequireFresh)

    Write-Host "SMOKE-STAGE: launching application from '$executable'"
    $jobProcess = [VhmReleaseSmoke.JobProcess]::Start(
        $executable,
        [System.IO.Path]::GetDirectoryName($executable),
        $childEnvironment,
        $standardOutputPath,
        $standardErrorPath
    )
    if ($jobProcess.RootHasExited()) {
        Write-Host "SMOKE-STAGE: application exited immediately (code $($jobProcess.GetRootExitCode()))"
        throw "Application exited immediately after its suspended, job-assigned launch with code $($jobProcess.GetRootExitCode())."
    }
    $activeJobProcessIds = @(Update-ObservedJobProcesses -Job $jobProcess -Observed $observedJobProcessIds)
    if ($activeJobProcessIds -notcontains $jobProcess.ProcessId) {
        throw "Windows Job did not report its live root process $($jobProcess.ProcessId)."
    }

    $startupDeadline = [datetime]::UtcNow.AddSeconds($StartupTimeoutSeconds)
    $startupReady = $false
    do {
        if ($jobProcess.RootHasExited()) {
            Write-Host "SMOKE-STAGE: application exited during startup (code $($jobProcess.GetRootExitCode()))"
            throw "Application exited during startup with code $($jobProcess.GetRootExitCode())."
        }
        [void] @(Update-ObservedJobProcesses -Job $jobProcess -Observed $observedJobProcessIds)
        if ($appWindowHandle -eq [IntPtr]::Zero) {
            $applicationWindows = @(Get-ApplicationWindows -ProcessId $jobProcess.ProcessId)
            $startupWindowInventory = @(ConvertTo-WindowInventoryReport -Windows $applicationWindows)
            $candidate = Select-MainApplicationWindow `
                -Windows $applicationWindows `
                -ProcessId $jobProcess.ProcessId
            if ($null -ne $candidate) {
                $appWindowInfo = $candidate
                $appWindowHandle = $candidate.Handle
                [VhmReleaseSmoke.NativeWindows]::Minimize($appWindowHandle)
            }
        }
        if ((Test-Path -LiteralPath $databasePath -PathType Leaf) -and
            (Test-Path -LiteralPath $thumbnailCachePath -PathType Container) -and
            (Test-Path -LiteralPath $webView2Path -PathType Container) -and
            $appWindowHandle -ne [IntPtr]::Zero) {
            $startupReady = $true
            break
        }
        Start-Sleep -Milliseconds 200
    } while ([datetime]::UtcNow -lt $startupDeadline)

    if (-not $startupReady) {
        Write-Host "SMOKE-STAGE: startup readiness timed out (window=$($appWindowHandle -ne [IntPtr]::Zero), db=$(Test-Path -LiteralPath $databasePath), cache=$(Test-Path -LiteralPath $thumbnailCachePath), webview2=$(Test-Path -LiteralPath $webView2Path))"
        throw "Application did not establish its visible main window, database, thumbnail cache, and WebView2 profile within $StartupTimeoutSeconds seconds. Root PID: $($jobProcess.ProcessId). Window inventory: $($startupWindowInventory | ConvertTo-Json -Depth 5 -Compress)"
    }
    Write-Host "SMOKE-STAGE: main window established (pid $($jobProcess.ProcessId))"

    [void] (Assert-IsolationRoot -Root $smokeRoot -Parent $isolationParentCanonical -ProtectedPaths $protectedPaths)
    foreach ($expectedPath in @($databasePath, $thumbnailCachePath, $webView2Path)) {
        $resolvedExpectedPath = Get-CanonicalExistingPath `
            -LiteralPath $expectedPath `
            -RequireDirectory ([System.IO.Directory]::Exists($expectedPath)) `
            -Description 'isolated runtime path'
        if (-not (Test-PathWithinRoot -Root $smokeRoot -Candidate $resolvedExpectedPath)) {
            throw "Runtime path escaped the smoke root: '$resolvedExpectedPath'."
        }
        $expectedItem = Get-Item -LiteralPath $resolvedExpectedPath -Force
        if (($expectedItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Runtime path must not be a reparse point: '$resolvedExpectedPath'."
        }
    }

    $windowBeforeSecondInstance = [VhmReleaseSmoke.NativeWindows]::Describe($appWindowHandle)
    if ($null -eq $windowBeforeSecondInstance -or
        -not [VhmReleaseSmoke.NativeWindows]::ExistsForProcess(
            $appWindowHandle,
            $jobProcess.ProcessId
        )) {
        throw 'The original main window disappeared before the single-instance handoff check.'
    }
    if (-not $windowBeforeSecondInstance.Visible -or -not $windowBeforeSecondInstance.Minimized) {
        throw 'The original main window was expected to be visible but minimized before the second launch; restore handoff cannot be proved.'
    }
    if (([string] $childEnvironment['VHM_RELEASE_SMOKE_ROOT']) -cne $smokeRoot -or
        ([string] $childEnvironment['WEBVIEW2_USER_DATA_FOLDER']) -cne $webView2Path) {
        throw 'The second launch environment does not reference the original marker-gated smoke and WebView2 roots.'
    }
    if ($jobProcess.RootHasExited()) {
        throw "The original application exited immediately before the second launch with code $($jobProcess.GetRootExitCode())."
    }

    $secondInstanceStartedAt = [datetime]::UtcNow
    $secondInstanceStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    Write-Host 'SMOKE-STAGE: launching second instance for handoff check'
    $secondInstanceProcess = [VhmReleaseSmoke.JobProcess]::Start(
        $executable,
        [System.IO.Path]::GetDirectoryName($executable),
        $childEnvironment,
        $secondInstanceStandardOutputPath,
        $secondInstanceStandardErrorPath
    )
    if ($secondInstanceProcess.ProcessId -eq $jobProcess.ProcessId) {
        throw 'The second launch unexpectedly reused the original process ID.'
    }
    $activeSecondInstanceProcessIds = @(
        Update-ObservedJobProcesses `
            -Job $secondInstanceProcess `
            -Observed $observedSecondInstanceProcessIds
    )
    if (-not $secondInstanceProcess.RootHasExited() -and
        $activeSecondInstanceProcessIds -notcontains $secondInstanceProcess.ProcessId) {
        throw "The second launch Job did not report its root process $($secondInstanceProcess.ProcessId)."
    }
    if (-not $secondInstanceProcess.WaitForRootExit($SecondInstanceTimeoutSeconds * 1000)) {
        throw "The second launch did not exit within $SecondInstanceTimeoutSeconds seconds; single-instance enforcement is unavailable."
    }
    $secondInstanceStopwatch.Stop()
    $secondInstanceExitedAt = [datetime]::UtcNow
    $secondInstanceExitMilliseconds = [long] $secondInstanceStopwatch.ElapsedMilliseconds
    $secondInstanceExitLimitMilliseconds = [long] $SecondInstanceTimeoutSeconds * 1000L
    if ($secondInstanceExitMilliseconds -gt $secondInstanceExitLimitMilliseconds) {
        throw "The second launch exited after $secondInstanceExitMilliseconds ms, exceeding the end-to-end $secondInstanceExitLimitMilliseconds ms single-instance deadline."
    }
    $secondInstanceExitCode = [uint32] $secondInstanceProcess.GetRootExitCode()
    if ($secondInstanceExitCode -ne 0) {
        throw "The second launch returned non-zero exit code $secondInstanceExitCode."
    }
    Assert-NoActiveJobProcesses `
        -Job $secondInstanceProcess `
        -Observed $observedSecondInstanceProcessIds `
        -Deadline ([datetime]::UtcNow.AddSeconds($SecondInstanceTimeoutSeconds))
    $secondInstanceAccounting = $secondInstanceProcess.GetAccounting()

    if ($jobProcess.RootHasExited()) {
        throw "The original application exited during the second launch with code $($jobProcess.GetRootExitCode())."
    }
    $primaryActiveProcessIdsAfterHandoff = @(
        Update-ObservedJobProcesses -Job $jobProcess -Observed $observedJobProcessIds
    )
    if ($primaryActiveProcessIdsAfterHandoff -notcontains $jobProcess.ProcessId) {
        throw "The original root process $($jobProcess.ProcessId) was no longer active after the second launch."
    }

    $windowRestoreDeadline = [datetime]::UtcNow.AddSeconds($SecondInstanceTimeoutSeconds)
    $windowAfterSecondInstance = $null
    do {
        $windowAfterSecondInstance = [VhmReleaseSmoke.NativeWindows]::Describe($appWindowHandle)
        if ($null -ne $windowAfterSecondInstance -and
            $windowAfterSecondInstance.Visible -and
            -not $windowAfterSecondInstance.Minimized) {
            break
        }
        Start-Sleep -Milliseconds 100
    } while ([datetime]::UtcNow -lt $windowRestoreDeadline)
    if ($null -eq $windowAfterSecondInstance -or
        [int] $windowAfterSecondInstance.ProcessId -ne $jobProcess.ProcessId -or
        -not $windowAfterSecondInstance.Visible -or
        $windowAfterSecondInstance.Minimized) {
        throw 'The second launch exited, but the original main window was not preserved, visible, and restored from minimization.'
    }

    $primaryWindowsAfterHandoff = @(Get-ApplicationWindows -ProcessId $jobProcess.ProcessId)
    $selectedWindowAfterHandoff = Select-MainApplicationWindow `
        -Windows $primaryWindowsAfterHandoff `
        -ProcessId $jobProcess.ProcessId
    if ($null -eq $selectedWindowAfterHandoff -or
        $selectedWindowAfterHandoff.Handle -ne $appWindowHandle) {
        throw 'The single-instance handoff did not preserve the original selected main window handle.'
    }

    $namedRootProcessIdsAfterHandoff = @()
    $namedRootDeadline = [datetime]::UtcNow.AddSeconds($SecondInstanceTimeoutSeconds)
    do {
        $namedRootProcessIdsAfterHandoff = @(Get-ProcessIdsByName -ProcessName $processName)
        if ($namedRootProcessIdsAfterHandoff.Count -eq 1 -and
            $namedRootProcessIdsAfterHandoff[0] -eq $jobProcess.ProcessId) {
            break
        }
        Start-Sleep -Milliseconds 100
    } while ([datetime]::UtcNow -lt $namedRootDeadline)
    if ($namedRootProcessIdsAfterHandoff.Count -ne 1 -or
        $namedRootProcessIdsAfterHandoff[0] -ne $jobProcess.ProcessId) {
        throw "Expected the original PID to be the only '$processName' root after handoff; found $([string]::Join(', ', $namedRootProcessIdsAfterHandoff))."
    }

    $foregroundWindowHandle = [VhmReleaseSmoke.NativeWindows]::ForegroundWindow()
    $foregroundMatchesMainWindow = ($foregroundWindowHandle -eq $appWindowHandle)
    $handleWidth = [IntPtr]::Size * 2
    $secondInstanceProcessOutputCapture = Get-ProcessOutputCapture `
        -StandardOutputPath $secondInstanceStandardOutputPath `
        -StandardErrorPath $secondInstanceStandardErrorPath `
        -EnvironmentRoot $environmentRoot
    $singleInstanceDiagnostics = [ordered]@{
        verified = $true
        sharedLaunchConfiguration = [ordered]@{
            executablePath = $executable
            executableSha256 = $executableHash
            workingDirectory = [System.IO.Path]::GetDirectoryName($executable)
            environmentOverrides = [ordered]@{
                VHM_RELEASE_SMOKE_ROOT = [string] $childEnvironment['VHM_RELEASE_SMOKE_ROOT']
                WEBVIEW2_USER_DATA_FOLDER = [string] $childEnvironment['WEBVIEW2_USER_DATA_FOLDER']
                TEMP = [string] $childEnvironment['TEMP']
                TMP = [string] $childEnvironment['TMP']
                VHM_FFMPEG_PATH = [string] $childEnvironment['VHM_FFMPEG_PATH']
                VHM_REAL_SCAN_DB = [string] $childEnvironment['VHM_REAL_SCAN_DB']
                VHM_REAL_SCAN_ROOT = [string] $childEnvironment['VHM_REAL_SCAN_ROOT']
                RUST_BACKTRACE = [string] $childEnvironment['RUST_BACKTRACE']
            }
        }
        secondInstanceExitTimeoutSeconds = $SecondInstanceTimeoutSeconds
        secondInstanceExitLimitMilliseconds = $secondInstanceExitLimitMilliseconds
        secondInstanceStartedAtUtc = $secondInstanceStartedAt.ToString('o')
        secondInstanceExitedAtUtc = $secondInstanceExitedAt.ToString('o')
        secondInstanceExitMilliseconds = $secondInstanceExitMilliseconds
        secondInstanceExitedWithinDeadline = $true
        primaryProcessId = [int] $jobProcess.ProcessId
        secondInstanceProcessId = [int] $secondInstanceProcess.ProcessId
        secondInstanceExitCode = $secondInstanceExitCode
        secondInstanceJobActiveProcessesAfterExit = [uint32] $secondInstanceAccounting.ActiveProcesses
        onlyPrimaryNamedRootAfterHandoff = $true
        namedRootProcessIdsAfterHandoff = @($namedRootProcessIdsAfterHandoff)
        primaryJobActiveProcessIdsAfterHandoff = @($primaryActiveProcessIdsAfterHandoff)
        primaryProcessAliveBeforeSecondLaunch = $true
        primaryProcessAliveAfterSecondExit = $true
        primaryWindowMinimizedBeforeHandoff = $true
        primaryWindowAliveAfterHandoff = $true
        primaryWindowHandlePreserved = $true
        primaryWindowVisibleAfterHandoff = $true
        primaryWindowMinimizedAfterHandoff = $false
        primaryWindowAfterHandoff = ConvertTo-WindowReport -Window $windowAfterSecondInstance
        primaryWindowInventoryAfterHandoff = @(
            ConvertTo-WindowInventoryReport -Windows $primaryWindowsAfterHandoff
        )
        foregroundWindowHandle = ('0x{0:X' + $handleWidth + '}') -f [uint64] $foregroundWindowHandle.ToInt64()
        foregroundMatchesMainWindow = $foregroundMatchesMainWindow
        focusVerification = if ($foregroundMatchesMainWindow) {
            'foreground-window-matched'
        }
        elseif ($foregroundWindowHandle -eq [IntPtr]::Zero) {
            'unavailable-on-window-station-non-gating'
        }
        else {
            'best-effort-not-matched-non-gating'
        }
        processOutput = $secondInstanceProcessOutputCapture
    }

    $inspection = Invoke-SqliteInspection `
        -PythonPath $python `
        -DatabasePath $databasePath `
        -TimeoutSeconds $DatabaseInspectionTimeoutSeconds `
        -IsolatedTemp $environmentPaths.temp
    Assert-DatabaseInspection -Inspection $inspection

    $survivalDeadline = [datetime]::UtcNow.AddSeconds(2)
    do {
        if ($jobProcess.RootHasExited()) {
            throw "Application exited before the startup survival interval completed with code $($jobProcess.GetRootExitCode())."
        }
        [void] @(Update-ObservedJobProcesses -Job $jobProcess -Observed $observedJobProcessIds)
        Start-Sleep -Milliseconds 200
    } while ([datetime]::UtcNow -lt $survivalDeadline)

    $closeStartedAt = [datetime]::UtcNow
    $windowBeforeClose = [VhmReleaseSmoke.NativeWindows]::Describe($appWindowHandle)
    $closeRequested = [VhmReleaseSmoke.NativeWindows]::RequestClose($appWindowHandle)
    $windowDisappearedAt = $null
    $activeJobProcessIdsAfterClose = @()
    $shutdownDeadline = $closeStartedAt.AddSeconds($ShutdownTimeoutSeconds)
    do {
        if ($null -eq $windowDisappearedAt -and
            -not [VhmReleaseSmoke.NativeWindows]::ExistsForProcess(
                $appWindowHandle,
                $jobProcess.ProcessId
            )) {
            $windowDisappearedAt = [datetime]::UtcNow
        }
        $activeJobProcessIdsAfterClose = @(
            Update-ObservedJobProcesses -Job $jobProcess -Observed $observedJobProcessIds
        )
        if ($jobProcess.RootHasExited()) {
            break
        }
        Start-Sleep -Milliseconds 100
    } while ([datetime]::UtcNow -lt $shutdownDeadline)

    $windowAfterClose = [VhmReleaseSmoke.NativeWindows]::Describe($appWindowHandle)
    if ($null -eq $windowDisappearedAt -and $null -eq $windowAfterClose) {
        $windowDisappearedAt = [datetime]::UtcNow
    }
    $windowsAfterClose = @(Get-ApplicationWindows -ProcessId $jobProcess.ProcessId)
    $shutdownAccounting = $jobProcess.GetAccounting()
    $closeDiagnostics = [ordered]@{
        rootProcessId = [int] $jobProcess.ProcessId
        selectedBeforeHide = ConvertTo-WindowReport -Window $appWindowInfo
        selectedBeforeClose = if ($null -ne $windowBeforeClose) {
            ConvertTo-WindowReport -Window $windowBeforeClose
        }
        else {
            $null
        }
        wmClosePosted = [bool] $closeRequested
        windowDisappeared = ($null -ne $windowDisappearedAt)
        windowDisappearedAfterMilliseconds = if ($null -ne $windowDisappearedAt) {
            [long] ($windowDisappearedAt - $closeStartedAt).TotalMilliseconds
        }
        else {
            $null
        }
        selectedAfterClose = if ($null -ne $windowAfterClose) {
            ConvertTo-WindowReport -Window $windowAfterClose
        }
        else {
            $null
        }
        rootExited = [bool] $jobProcess.RootHasExited()
        elapsedMilliseconds = [long] ([datetime]::UtcNow - $closeStartedAt).TotalMilliseconds
        activeJobProcessIds = @($activeJobProcessIdsAfterClose)
        jobAccounting = [ordered]@{
            totalProcesses = [uint32] $shutdownAccounting.TotalProcesses
            activeProcesses = [uint32] $shutdownAccounting.ActiveProcesses
            totalTerminatedProcesses = [uint32] $shutdownAccounting.TotalTerminatedProcesses
        }
        rootWindowInventoryAfterClose = @(
            ConvertTo-WindowInventoryReport -Windows $windowsAfterClose
        )
    }

    if (-not $closeRequested) {
        throw "Posting WM_CLOSE to the selected main window failed. Diagnostics: $($closeDiagnostics | ConvertTo-Json -Depth 8 -Compress)"
    }
    if (-not $jobProcess.RootHasExited()) {
        $shutdownFailure = if ($null -eq $windowDisappearedAt) {
            'The selected main HWND remained alive after WM_CLOSE.'
        }
        else {
            'The selected main HWND disappeared, but the Tauri root process/event loop remained alive.'
        }
        throw "$shutdownFailure The application did not exit gracefully within $ShutdownTimeoutSeconds seconds. Diagnostics: $($closeDiagnostics | ConvertTo-Json -Depth 8 -Compress)"
    }
    $rootExitCode = [uint32] $jobProcess.GetRootExitCode()
    if ($rootExitCode -ne 0) {
        throw "Application returned non-zero exit code $rootExitCode after graceful shutdown."
    }

    Assert-NoActiveJobProcesses `
        -Job $jobProcess `
        -Observed $observedJobProcessIds `
        -Deadline ([datetime]::UtcNow.AddSeconds($ShutdownTimeoutSeconds))
    $jobAccounting = $jobProcess.GetAccounting()

    Write-Host 'SMOKE-STAGE: verifying real application data/cache trees are unchanged'
    $realDataAfter = Get-TreeFingerprint -Root $realPaths.data -Description 'real application data directory'
    $realCacheAfter = Get-TreeFingerprint -Root $realPaths.cache -Description 'real application cache directory'
    $realStateChecked = $true
    if (-not (Test-FingerprintEqual -Before $realDataBefore -After $realDataAfter)) {
        throw "Real application data changed during isolated startup: '$($realPaths.data)'. Before: $(Format-FingerprintSummary $realDataBefore). After: $(Format-FingerprintSummary $realDataAfter)."
    }
    if (-not (Test-FingerprintEqual -Before $realCacheBefore -After $realCacheAfter)) {
        throw "Real application cache changed during isolated startup: '$($realPaths.cache)'. Before: $(Format-FingerprintSummary $realCacheBefore). After: $(Format-FingerprintSummary $realCacheAfter)."
    }
    Write-Host "SMOKE-STAGE: real trees fingerprinted and unchanged (data exists=$($realDataAfter.exists), cache exists=$($realCacheAfter.exists))"

    $report = [ordered]@{
        status = 'passed'
        checkedAtUtc = [datetime]::UtcNow.ToString('o')
        durationMilliseconds = [long] ([datetime]::UtcNow - $startedAt).TotalMilliseconds
        executable = [ordered]@{
            path = $executable
            sha256 = $executableHash
            exitCode = $rootExitCode
        }
        runtime = [ordered]@{
            isolationRoot = $smokeRoot
            environmentRoot = $environmentRoot
            databasePath = $databasePath
            thumbnailCachePath = $thumbnailCachePath
            webView2UserDataPath = $webView2Path
            ffmpegMode = $FfmpegMode
            processContainment = 'windows-job-kill-on-close'
            singleInstance = $singleInstanceDiagnostics
            selectedMainWindow = ConvertTo-WindowReport -Window $appWindowInfo
            startupWindowInventory = @($startupWindowInventory)
            gracefulClose = $closeDiagnostics
            observedProcessCount = $observedJobProcessIds.Count
            jobTotalAssignedProcesses = [uint32] $jobAccounting.TotalProcesses
            jobActiveProcessesAfterShutdown = [uint32] $jobAccounting.ActiveProcesses
            jobTotalTerminatedProcesses = [uint32] $jobAccounting.TotalTerminatedProcesses
        }
        database = [ordered]@{
            schemaVersion = [int] $inspection.schemaVersion
            quickCheck = [string] $inspection.quickCheck
            requiredTableCount = $RequiredTables.Count
            requiredTables = @($RequiredTables)
            actualTables = @($inspection.tables)
            requiredColumns = $RequiredColumns
            actualColumns = $inspection.columns
            customTagCount = @($inspection.tags).Count
            customTagVerification = 'fresh-database-empty-user-tag-catalog'
            scanRunCount = [long] $inspection.counts.scan_runs
            sourceDirectoryCount = [long] $inspection.counts.source_dirs
            clipCount = [long] $inspection.counts.clips
            trashSnapshotCount = [long] $inspection.counts.clip_trash_snapshots
            trashSnapshotVerification = 'fresh-database-empty-trash-identity-snapshot-catalog'
            deleteIntentCount = [long] $inspection.counts.clip_delete_intents
            deleteIntentVerification = 'fresh-database-empty-delete-intent-journal'
        }
        realApplicationState = [ordered]@{
            unchanged = $true
            data = [ordered]@{
                path = $realPaths.data
                before = (ConvertTo-FingerprintReport $realDataBefore)
                after = (ConvertTo-FingerprintReport $realDataAfter)
                unchanged = $true
            }
            cache = [ordered]@{
                path = $realPaths.cache
                before = (ConvertTo-FingerprintReport $realCacheBefore)
                after = (ConvertTo-FingerprintReport $realCacheAfter)
                unchanged = $true
            }
        }
    }
}
catch {
    $primaryError = $_
    Write-Host "SMOKE-STAGE: failure captured: $($_.Exception.Message)"
}
finally {
    if ($null -ne $secondInstanceProcess) {
        try {
            $activeSecondInstanceProcessIds = @(
                Update-ObservedJobProcesses `
                    -Job $secondInstanceProcess `
                    -Observed $observedSecondInstanceProcessIds
            )
            if ($activeSecondInstanceProcessIds.Count -gt 0) {
                $secondInstanceProcess.Terminate(1)
                Assert-NoActiveJobProcesses `
                    -Job $secondInstanceProcess `
                    -Observed $observedSecondInstanceProcessIds `
                    -Deadline ([datetime]::UtcNow.AddSeconds($SecondInstanceTimeoutSeconds))
            }
        }
        catch {
            if ($null -eq $cleanupError) {
                $cleanupError = $_
            }
        }
        finally {
            $secondInstanceProcess.Dispose()
        }
    }

    if ($null -ne $jobProcess) {
        try {
            $activeJobProcessIds = @(Update-ObservedJobProcesses -Job $jobProcess -Observed $observedJobProcessIds)
            if ($activeJobProcessIds.Count -gt 0) {
                $jobProcess.Terminate(1)
                Assert-NoActiveJobProcesses `
                    -Job $jobProcess `
                    -Observed $observedJobProcessIds `
                    -Deadline ([datetime]::UtcNow.AddSeconds($ShutdownTimeoutSeconds))
            }
        }
        catch {
            if ($null -eq $cleanupError) {
                $cleanupError = $_
            }
        }
        finally {
            # Closing a KILL_ON_JOB_CLOSE handle is the final containment boundary even if a query
            # or explicit termination failed. Such a failure is still reported above.
            $jobProcess.Dispose()
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($environmentRoot) -and
        (Test-Path -LiteralPath $environmentRoot) -and
        -not [string]::IsNullOrWhiteSpace($standardOutputPath) -and
        -not [string]::IsNullOrWhiteSpace($standardErrorPath)) {
        try {
            $processOutputCapture = Get-ProcessOutputCapture `
                -StandardOutputPath $standardOutputPath `
                -StandardErrorPath $standardErrorPath `
                -EnvironmentRoot $environmentRoot
        }
        catch {
            if ($null -eq $cleanupError) {
                $cleanupError = $_
            }
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($environmentRoot) -and
        (Test-Path -LiteralPath $environmentRoot) -and
        -not [string]::IsNullOrWhiteSpace($secondInstanceStandardOutputPath) -and
        -not [string]::IsNullOrWhiteSpace($secondInstanceStandardErrorPath)) {
        try {
            $secondInstanceProcessOutputCapture = Get-ProcessOutputCapture `
                -StandardOutputPath $secondInstanceStandardOutputPath `
                -StandardErrorPath $secondInstanceStandardErrorPath `
                -EnvironmentRoot $environmentRoot
        }
        catch {
            if ($null -eq $cleanupError) {
                $cleanupError = $_
            }
        }
    }

    if (-not $realStateChecked) {
        try {
            $realDataAfter = Get-TreeFingerprint -Root $realPaths.data -Description 'real application data directory'
            $realCacheAfter = Get-TreeFingerprint -Root $realPaths.cache -Description 'real application cache directory'
            if (-not (Test-FingerprintEqual -Before $realDataBefore -After $realDataAfter)) {
                throw "Real application data changed during failed isolated startup: '$($realPaths.data)'. Before: $(Format-FingerprintSummary $realDataBefore). After: $(Format-FingerprintSummary $realDataAfter)."
            }
            if (-not (Test-FingerprintEqual -Before $realCacheBefore -After $realCacheAfter)) {
                throw "Real application cache changed during failed isolated startup: '$($realPaths.cache)'. Before: $(Format-FingerprintSummary $realCacheBefore). After: $(Format-FingerprintSummary $realCacheAfter)."
            }
            $realStateChecked = $true
        }
        catch {
            if ($null -eq $cleanupError) {
                $cleanupError = $_
            }
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($smokeRoot) -and (Test-Path -LiteralPath $smokeRoot)) {
        try {
            $safeRoot = Assert-IsolationRoot `
                -Root $smokeRoot `
                -Parent $isolationParentCanonical `
                -ProtectedPaths @($protectedPaths + @($environmentRoot | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })) `
                -InspectAllChildren
            Remove-Item -LiteralPath $safeRoot -Recurse -Force -ErrorAction Stop
            if (Test-Path -LiteralPath $safeRoot) {
                throw "Release smoke root remained after cleanup: '$safeRoot'."
            }
        }
        catch {
            if ($null -eq $cleanupError) {
                $cleanupError = $_
            }
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($environmentRoot) -and (Test-Path -LiteralPath $environmentRoot)) {
        try {
            $safeEnvironmentRoot = Assert-EnvironmentRoot `
                -Root $environmentRoot `
                -Parent $isolationParentCanonical `
                -ProtectedPaths @($protectedPaths + $smokeRoot) `
                -InspectAllChildren
            Remove-Item -LiteralPath $safeEnvironmentRoot -Recurse -Force -ErrorAction Stop
            if (Test-Path -LiteralPath $safeEnvironmentRoot) {
                throw "Release smoke environment root remained after cleanup: '$safeEnvironmentRoot'."
            }
        }
        catch {
            if ($null -eq $cleanupError) {
                $cleanupError = $_
            }
        }
    }
}

if ($null -ne $primaryError) {
    $primaryProcessOutputEvidence = if ($null -ne $processOutputCapture) {
        Format-ProcessOutputEvidence -Capture $processOutputCapture
    }
    else {
        'process output was unavailable'
    }
    $secondProcessOutputEvidence = if ($null -ne $secondInstanceProcessOutputCapture) {
        Format-ProcessOutputEvidence -Capture $secondInstanceProcessOutputCapture
    }
    else {
        'second-instance process output was unavailable'
    }
    $failureMessage = "Release smoke failed: $($primaryError.Exception.Message) Primary process output:`n$primaryProcessOutputEvidence`nSecond-instance process output:`n$secondProcessOutputEvidence"
    if ($null -ne $cleanupError) {
        throw [System.InvalidOperationException]::new(
            "$failureMessage Cleanup/safety verification also failed: $($cleanupError.Exception.Message)",
            $primaryError.Exception
        )
    }
    throw [System.InvalidOperationException]::new($failureMessage, $primaryError.Exception)
}
if ($null -ne $cleanupError) {
    throw $cleanupError
}

$report.runtime['processOutput'] = $processOutputCapture
$report.runtime.singleInstance['processOutput'] = $secondInstanceProcessOutputCapture
$report.runtime['cleaned'] = $true
$global:LASTEXITCODE = 0
Write-Host 'SMOKE-STAGE: emitting report'
try {
    $report | ConvertTo-Json -Depth 12
    Write-Host 'SMOKE-STAGE: report emitted'
}
catch {
    Write-Host "SMOKE-STAGE: report emission failed: $($_.Exception.Message)"
    throw
}
