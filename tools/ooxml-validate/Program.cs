// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
// SPDX-License-Identifier: AGPL-3.0-or-later

// The Microsoft-side oracle: Open XML SDK's schema and semantic validator, which is the closest
// thing to "Office will open this" that runs on Linux. Prints one line per error as
// `<file>\t<part>\t<path>\t<id>\t<description>` and exits 1 when any file has an error, 2 when a
// file cannot be opened as a package at all.
using DocumentFormat.OpenXml;
using DocumentFormat.OpenXml.Packaging;
using DocumentFormat.OpenXml.Validation;

var version = FileFormatVersions.Office2007;
var files = new List<string>();
foreach (var arg in args)
{
    if (arg.StartsWith("--version="))
        version = Enum.Parse<FileFormatVersions>(arg["--version=".Length..]);
    else
        files.Add(arg);
}
if (files.Count == 0)
{
    Console.Error.WriteLine("usage: ooxml-validate [--version=Office2007|Office2010|...|Microsoft365] FILE...");
    return 64;
}

var status = 0;
var validator = new OpenXmlValidator(version);
foreach (var file in files)
{
    try
    {
        using OpenXmlPackage doc = Path.GetExtension(file).ToLowerInvariant() switch
        {
            ".xlsx" => SpreadsheetDocument.Open(file, false),
            ".docx" => WordprocessingDocument.Open(file, false),
            var ext => throw new InvalidDataException($"unknown extension {ext}"),
        };
        foreach (var e in validator.Validate(doc))
        {
            status = Math.Max(status, 1);
            Console.WriteLine($"{file}\t{e.Part?.Uri}\t{e.Path?.XPath}\t{e.Id}\t{e.Description}");
        }
    }
    catch (Exception e)
    {
        status = 2;
        Console.WriteLine($"{file}\t\t\tOPEN\t{e.GetType().Name}: {e.Message}");
    }
}
return status;
