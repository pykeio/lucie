std::arch::global_asm!(
	r#".pushsection .rsrc,"dw""#,
	"2:",
	".zero 14",
	".short 1",                     // number of entries
	".long 24",                     // RT_MANIFEST
	".long (3f - 2b) | 0x80000000", // offset to next table
	"3:",
	".zero 14",
	".short 1",                     // number of entries
	".long 1",                      // process manifest (ID 1)
	".long (4f - 2b) | 0x80000000", // offset to next table
	"4:",
	".zero 14",
	".short 1",        // number of entries
	".long 1033",      // US English LCID (0x0409)
	".long (5f - 2b)", // offset to data entry
	"5:",
	".long 6f@imgrel", // pointer to manifest data
	".long (7f - 6f)", // length of manifest data
	".zero 8",
	".p2align 2",
	"6:",
	r#".ascii "<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">""#,
	r#".ascii "    <trustInfo xmlns=\"urn:schemas-microsoft-com:asm.v3\">""#,
	r#".ascii "        <security>""#,
	r#".ascii "            <requestedPrivileges>""#,
	r#".ascii "                <requestedExecutionLevel level=\"asInvoker\" uiAccess=\"false\" />""#,
	r#".ascii "            </requestedPrivileges>""#,
	r#".ascii "        </security>""#,
	r#".ascii "    </trustInfo>""#,
	r#".ascii "    <compatibility xmlns=\"urn:schemas-microsoft-com:compatibility.v1\">""#,
	r#".ascii "        <application>""#,
	r#".ascii "            <!-- Windows 10 -->""#,
	r#".ascii "            <supportedOS Id=\"{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}\" />""#,
	r#".ascii "        </application>""#,
	r#".ascii "    </compatibility>""#,
	r#".ascii "    <application xmlns=\"urn:schemas-microsoft-com:asm.v3\">""#,
	r#".ascii "        <windowsSettings>""#,
	r#".ascii "            <dpiAware xmlns=\"http://schemas.microsoft.com/SMI/2005/WindowsSettings\">true/pm</dpiAware>""#,
	r#".ascii "            <dpiAwareness xmlns=\"http://schemas.microsoft.com/SMI/2016/WindowsSettings\">PerMonitorV2</dpiAwareness>""#,
	r#".ascii "        </windowsSettings>""#,
	r#".ascii "    </application>""#,
	r#".ascii "    <dependency>""#,
	r#".ascii "        <dependentAssembly>""#,
	r#".ascii "            <assemblyIdentity""#,
	r#".ascii "                type='win32'""#,
	r#".ascii "                name='Microsoft.Windows.Common-Controls'""#,
	r#".ascii "                version='6.0.0.0'""#,
	r#".ascii "                processorArchitecture='*'""#,
	r#".ascii "                publicKeyToken='6595b64144ccf1df'""#,
	r#".ascii "            />""#,
	r#".ascii "        </dependentAssembly>""#,
	r#".ascii "    </dependency>""#,
	r#".ascii "</assembly>""#,
	"7:",
	".p2align 2",
	".popsection"
);
