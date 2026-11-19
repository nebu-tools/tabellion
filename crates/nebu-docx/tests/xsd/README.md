# ECMA-376 transitional schemas

These are XML Schema files of ECMA-376 Office Open XML File Formats, fifth
edition (December 2016), Part 4, Transitional Migration Features, from the
archive `OfficeOpenXML-XMLSchema-Transitional.zip` in
`ECMA-376-4_5th_edition_december_2016.zip` at
<https://ecma-international.org/publications-and-standards/standards/ecma-376/>.
They are copied unmodified, byte for byte: `wml.xsd`, `dml-main.xsd`,
`dml-wordprocessingDrawing.xsd`, `dml-picture.xsd`, `shared-math.xsd`,
`shared-commonSimpleTypes.xsd`, `shared-relationshipReference.xsd` and
`shared-customXmlSchemaProperties.xsd`.

The tests use them to validate every document they write
(`tests/support/xsd.rs`). They are test data: no build of the engine or of
the command line includes them.

They are distributed under Ecma International's copyright notice for its
standards, which allows copying and distributing them unmodified with the
notice:

> COPYRIGHT NOTICE
>
> © Ecma International
>
> By obtaining and/or copying this work, you (the licensee) agree that you
> have read, understood, and will comply with the following terms and
> conditions.
>
> This document may be copied, published and distributed to others, and
> certain derivative works of it may be prepared, copied, published, and
> distributed, in whole or in part, provided that the above copyright notice
> and this Copyright License and Disclaimer are included on all such copies
> and derivative works. The only derivative works that are permissible under
> this Copyright License and Disclaimer are:
>
> (i) works which incorporate all or portion of this document for the
> purpose of providing commentary or explanation (such as an annotated
> version of the document),
>
> (ii) works which incorporate all or portion of this document for the
> purpose of incorporating features that provide accessibility,
>
> (iii) translations of this document into languages other than English and
> into different formats and
>
> (iv) works by making use of this specification in standard conformant
> products by implementing (e.g. by copy and paste wholly or partly) the
> functionality therein.
>
> However, the content of this document itself may not be modified in any
> way, including by removing the copyright notice or references to Ecma
> International, except as required to translate it into languages other
> than English or into a different format.
>
> The official version of an Ecma International document is the English
> language version on the Ecma International website. In the event of
> discrepancies between a translated version and the official version, the
> official version shall govern.
>
> The limited permissions granted above are perpetual and will not be
> revoked by Ecma International or its successors or assigns.
>
> This document and the information contained herein is provided on an
> "AS IS" basis and ECMA INTERNATIONAL DISCLAIMS ALL WARRANTIES, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTY THAT THE USE OF THE
> INFORMATION HEREIN WILL NOT INFRINGE ANY OWNERSHIP RIGHTS OR ANY IMPLIED
> WARRANTIES OF MERCHANTABILITY OR FITNESS FOR A PARTICULAR PURPOSE.

The rest of this repository is under the GNU Affero General Public License,
version 3. These files keep their own notice above; the licence of the
repository does not apply to them.
